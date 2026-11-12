//! Migration 0237 grants `member:self` to every stored capability list that
//! holds `workspace:read`, on both backends: tokens and the grants later
//! tokens are minted from. A list without `workspace:read` is left alone, the
//! new entry goes last, and a second run changes nothing.

use chrono::{Duration, Utc};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    ApiTokenId, DelegationGrantId, MemberKind, NewApiToken, NewDelegationGrant, NewMember,
    NewWorkspace,
};

const SQLITE_0237: &str =
    include_str!("../../../migrations/sqlite/0237_member_self_capability.sql");
const POSTGRES_0237: &str =
    include_str!("../../../migrations/postgres/0237_member_self_capability.sql");

const READ: &str = "workspace:read";
const SELF: &str = "member:self";

struct Seeded {
    worker: ApiTokenId,
    reader: ApiTokenId,
    searcher: ApiTokenId,
    grant: DelegationGrantId,
}

fn caps(list: &[&str]) -> Vec<String> {
    list.iter().map(|c| c.to_string()).collect()
}

/// Rows as a live instance holds them: written with the lists they were
/// minted with before `member:self` existed.
async fn seed(store: &dyn Store) -> Seeded {
    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .expect("ws");
    let mut members = Vec::new();
    for handle in ["subject", "delegate"] {
        members.push(
            store
                .create_member(NewMember {
                    workspace_id: ws.id,
                    handle: handle.into(),
                    display_name: None,
                    kind: MemberKind::Agent,
                })
                .await
                .expect(handle),
        );
    }
    let token = |list: &'static [&'static str], hash: &'static str| {
        let member = members[0].id;
        async move {
            store
                .create_api_token(NewApiToken {
                    workspace_id: ws.id,
                    member_id: member,
                    app_installation_id: None,
                    token_hash: hash.repeat(64),
                    label: None,
                    capabilities: caps(list),
                    expires_at: None,
                })
                .await
                .expect("token")
                .id
        }
    };
    let worker = token(&[READ, "workspace:write", "message:post"], "a").await;
    let reader = token(&[READ], "b").await;
    let searcher = token(&["search:query"], "c").await;
    let grant = store
        .create_delegation_grant(NewDelegationGrant {
            workspace_id: ws.id,
            subject_id: members[0].id,
            delegate_id: members[1].id,
            capabilities: caps(&[READ, "thread:transition"]),
            purpose: "work for the subject".into(),
            authorized_by: members[0].id,
            expires_at: Utc::now() + Duration::hours(1),
        })
        .await
        .expect("grant")
        .id;
    Seeded {
        worker,
        reader,
        searcher,
        grant,
    }
}

async fn assert_granted(store: &dyn Store, s: &Seeded) {
    let token = |id| async move { store.get_api_token(id).await.expect("token").capabilities };
    assert_eq!(
        token(s.worker).await,
        caps(&[READ, "workspace:write", "message:post", SELF])
    );
    assert_eq!(token(s.reader).await, caps(&[READ, SELF]));
    assert_eq!(token(s.searcher).await, caps(&["search:query"]));
    // A stored grant keeps its list sorted, so compare it as a set.
    let mut grant = store
        .get_delegation_grant(s.grant)
        .await
        .expect("grant")
        .capabilities;
    grant.sort();
    let mut want = caps(&[READ, "thread:transition", SELF]);
    want.sort();
    assert_eq!(grant, want);
}

#[tokio::test]
async fn the_0237_fix_grants_member_self_to_every_workspace_read_list_sqlite() {
    let pool = maidan_search::sqlite_pool_options()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("pragma");
    run_sqlite_migrations(&pool).await.expect("migrate");
    let store = SqliteStore::for_tests(pool.clone());
    let s = seed(&store).await;

    sqlx::raw_sql(SQLITE_0237)
        .execute(&pool)
        .await
        .expect("run 0237");
    assert_granted(&store, &s).await;
    sqlx::raw_sql(SQLITE_0237)
        .execute(&pool)
        .await
        .expect("run 0237 again");
    assert_granted(&store, &s).await;
}

#[tokio::test]
async fn the_0237_fix_grants_member_self_to_every_workspace_read_list_postgres() {
    use maidan_store::{run_postgres_migrations, PostgresStore};
    use sqlx::postgres::PgPoolOptions;
    use std::time::Duration as StdDuration;
    use testcontainers::{runners::AsyncRunner, ImageExt};
    use testcontainers_modules::postgres::Postgres;

    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            maidan_store::test_support::docker::skip_start_failure(err).await;
            return;
        }
    };
    let host = container.get_host().await.expect("host");
    let port = container.get_host_port_ipv4(5432).await.expect("port");
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(StdDuration::from_secs(15))
        .connect(&url)
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    let store = PostgresStore::for_tests(pool.clone());
    let s = seed(&store).await;

    sqlx::raw_sql(POSTGRES_0237)
        .execute(&pool)
        .await
        .expect("run 0237");
    assert_granted(&store, &s).await;
    sqlx::raw_sql(POSTGRES_0237)
        .execute(&pool)
        .await
        .expect("run 0237 again");
    assert_granted(&store, &s).await;
}

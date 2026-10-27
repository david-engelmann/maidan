//! A removed (tombstoned) member's API tokens and agent keys stop resolving
//! at the bearer lookup, whatever tombstoned it (an import or a SCIM delete),
//! and another tenant's tokens keep working. A token whose member belongs to
//! another workspace does not resolve either. Two tenants, both backends.

use maidan_store::{prelude::*, run_sqlite_migrations, StoreError};
use maidan_types::{MemberId, MemberKind, NewApiToken, NewMember, NewWorkspace, WorkspaceId};
use sqlx::sqlite::SqlitePoolOptions;

struct Tenant {
    workspace: WorkspaceId,
    member: MemberId,
    token_hash: String,
}

async fn tenant(store: &dyn Store, name: &str, fill: char) -> Tenant {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: format!("{name}-agent"),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let token_hash = fill.to_string().repeat(64);
    store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: member.id,
            app_installation_id: None,
            token_hash: token_hash.clone(),
            label: Some("agent key".into()),
            capabilities: vec!["workspace:read".into()],
            expires_at: None,
        })
        .await
        .unwrap();
    Tenant {
        workspace: ws.id,
        member: member.id,
        token_hash,
    }
}

async fn setup(store: &dyn Store) -> (Tenant, Tenant, String) {
    let a = tenant(store, "alpha", 'a').await;
    let b = tenant(store, "beta", 'b').await;
    // Minted in beta, naming alpha's member: never a valid pairing.
    let cross = "c".repeat(64);
    let made = store
        .create_api_token(NewApiToken {
            workspace_id: b.workspace,
            member_id: a.member,
            app_installation_id: None,
            token_hash: cross.clone(),
            label: None,
            capabilities: vec!["workspace:read".into()],
            expires_at: None,
        })
        .await;
    // A store that refuses the pairing outright is just as good.
    let cross = if made.is_ok() { cross } else { String::new() };
    assert!(store
        .get_active_api_token_by_hash(&a.token_hash)
        .await
        .is_ok());
    assert!(store
        .get_active_api_token_by_hash(&b.token_hash)
        .await
        .is_ok());
    (a, b, cross)
}

async fn check(store: &dyn Store, a: &Tenant, b: &Tenant, cross: &str) {
    let err = store
        .get_active_api_token_by_hash(&a.token_hash)
        .await
        .expect_err("a tombstoned member's token must not resolve");
    assert!(matches!(err, StoreError::NotFound), "{err:?}");
    let other = store
        .get_active_api_token_by_hash(&b.token_hash)
        .await
        .expect("the other tenant's token still resolves");
    assert_eq!(other.workspace_id, b.workspace);
    assert_eq!(other.member_id, b.member);
    if !cross.is_empty() {
        let err = store
            .get_active_api_token_by_hash(cross)
            .await
            .expect_err("a token naming another workspace's member must not resolve");
        assert!(matches!(err, StoreError::NotFound), "{err:?}");
    }
}

#[tokio::test]
async fn a_tombstoned_members_tokens_stop_resolving_and_the_other_tenants_do_not_on_sqlite() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store = SqliteStore::for_tests(pool.clone());
    let (a, b, cross) = setup(&store).await;
    sqlx::query("UPDATE maidan_members SET tombstoned_at = ? WHERE id = ?")
        .bind(chrono::Utc::now())
        .bind(a.member.0)
        .execute(&pool)
        .await
        .unwrap();
    check(&store, &a, &b, &cross).await;
}

#[tokio::test]
async fn a_tombstoned_members_tokens_stop_resolving_and_the_other_tenants_do_not_on_postgres() {
    use maidan_store::{run_postgres_migrations, PostgresStore};
    use sqlx::postgres::PgPoolOptions;
    use std::time::Duration;
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
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .unwrap();
    run_postgres_migrations(&pool).await.unwrap();
    let store = PostgresStore::for_tests(pool.clone());
    let (a, b, cross) = setup(&store).await;
    sqlx::query("UPDATE maidan_members SET tombstoned_at = NOW() WHERE id = $1")
        .bind(a.member.0)
        .execute(&pool)
        .await
        .unwrap();
    check(&store, &a, &b, &cross).await;
}

//! Durable member last-seen (Cluster 252, Arc I): touch (upsert) / get. Both
//! backends. No wiring yet.

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberKind, NewMember, NewWorkspace};
use sqlx::sqlite::SqlitePoolOptions;

async fn sqlite() -> SqliteStore {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("pragma");
    run_sqlite_migrations(&pool).await.expect("migrate");
    SqliteStore::new(pool)
}

async fn run_suite(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace { name: "ls".into() })
        .await
        .expect("ws");
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "m".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("member");

    // Never seen -> None.
    assert!(store
        .get_member_last_seen(member.id)
        .await
        .expect("get none")
        .is_none());

    // Touch, then get returns a recent instant.
    store
        .touch_member_last_seen(member.id)
        .await
        .expect("touch");
    let first = store
        .get_member_last_seen(member.id)
        .await
        .expect("get")
        .expect("some");
    let age = chrono::Utc::now().signed_duration_since(first);
    assert!(
        age.num_seconds().abs() < 60,
        "last_seen is recent (age {age})"
    );

    // A re-touch advances (or holds) the timestamp — never goes backwards.
    store
        .touch_member_last_seen(member.id)
        .await
        .expect("re-touch");
    let second = store
        .get_member_last_seen(member.id)
        .await
        .expect("get2")
        .expect("some2");
    assert!(second >= first, "re-touch does not go backwards");
}

#[tokio::test]
async fn member_last_seen_touch_get_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn member_last_seen_touch_get_postgres() {
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
            eprintln!("skipping: docker unavailable ({err})");
            return;
        }
    };
    let host = container.get_host().await.expect("host");
    let port = container.get_host_port_ipv4(5432).await.expect("port");
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");
    let store = PostgresStore::new(pool);
    run_suite(&store).await;
}

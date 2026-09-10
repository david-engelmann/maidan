//! Thread dispatch-priority store (Cluster 365, G3 fair dispatch): set/get + the
//! default-0 (row-absent) contract. Both backends. The aged-rank `claim_next`
//! ordering is exercised in `thread_priority_dispatch.rs` (Cluster 365.2).

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberKind, NewChannel, NewMember, NewThread, NewWorkspace};
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
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .expect("ws");
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "a".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("member");
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "c".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("ch");
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("t".into()),
        })
        .await
        .expect("thread");

    // Default: no row = the implicit priority 0.
    assert!(store
        .get_thread_priority(thread.id)
        .await
        .expect("get")
        .is_none());

    // Set a priority; it reads back.
    let set = store
        .set_thread_priority(thread.id, 5, member.id)
        .await
        .expect("set");
    assert_eq!(set.priority, 5);
    assert_eq!(set.thread_id, thread.id);
    assert_eq!(set.set_by, member.id);
    let got = store
        .get_thread_priority(thread.id)
        .await
        .expect("get")
        .expect("priority");
    assert_eq!(got.priority, 5);

    // Upsert: re-setting replaces the value (one row per thread), incl. negatives.
    let reset = store
        .set_thread_priority(thread.id, -3, member.id)
        .await
        .expect("reset");
    assert_eq!(reset.priority, -3);
    assert_eq!(
        store
            .get_thread_priority(thread.id)
            .await
            .expect("get")
            .expect("priority")
            .priority,
        -3
    );
}

#[tokio::test]
async fn thread_priority_set_and_get_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn thread_priority_set_and_get_postgres() {
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
            eprintln!("skipping: docker unavailable ({err})");
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
    let store = PostgresStore::new(pool);
    run_suite(&store).await;
}

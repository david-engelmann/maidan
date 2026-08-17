//! Task structured results (Cluster 234, Arc F): set (upsert) / get a thread's
//! result. Both backends. No routes yet.

use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::{MemberKind, NewChannel, NewMember, NewThread, NewWorkspace};
use serde_json::json;
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
        .create_workspace(NewWorkspace {
            name: "results".into(),
        })
        .await
        .expect("ws");
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "agent".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("member");
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "tasks".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("ch");
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("task".into()),
        })
        .await
        .expect("thread");

    // No result until one is produced.
    assert!(store
        .get_thread_result(thread.id)
        .await
        .expect("get none")
        .is_none());

    // Set a structured result.
    let payload = json!({ "status": "ok", "score": 42, "items": ["a", "b"] });
    let set = store
        .set_thread_result(thread.id, member.id, &payload)
        .await
        .expect("set");
    assert_eq!(set.thread_id, thread.id);
    assert_eq!(set.produced_by, member.id);
    assert_eq!(set.result, payload);

    // get round-trips the JSON.
    let got = store
        .get_thread_result(thread.id)
        .await
        .expect("get")
        .expect("some");
    assert_eq!(got.result, payload);
    assert_eq!(got.produced_by, member.id);

    // A re-set overwrites (one result per thread).
    let payload2 = json!({ "status": "revised", "score": 100 });
    store
        .set_thread_result(thread.id, member.id, &payload2)
        .await
        .expect("re-set");
    let got2 = store
        .get_thread_result(thread.id)
        .await
        .expect("get2")
        .expect("some2");
    assert_eq!(got2.result, payload2, "re-set overwrites the prior result");
}

#[tokio::test]
async fn thread_result_set_get_upsert_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn thread_result_set_get_upsert_postgres() {
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

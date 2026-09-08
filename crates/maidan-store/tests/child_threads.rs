//! Collapsed child threads (Cluster 356, F2): a parent thread's child threads
//! with a live per-child message count, tombstoned children/messages excluded.

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberKind, NewChannel, NewMember, NewMessage, NewThread, NewWorkspace, ThreadId,
};
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

async fn run_child_suite(store: &dyn Store) {
    let ws = store
        .create_workspace(NewWorkspace { name: "cc".into() })
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
            name: "work".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("ch");
    let parent = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("parent".into()),
        })
        .await
        .expect("parent");
    let mk_child = |title: &str| NewThread {
        channel_id: channel.id,
        parent_thread_id: Some(parent.id),
        title: Some(title.into()),
    };
    let child1 = store.create_thread(mk_child("c1")).await.expect("c1");
    let child2 = store.create_thread(mk_child("c2")).await.expect("c2");

    let msg = |thread_id: ThreadId| NewMessage {
        thread_id,
        author_id: member.id,
        body: "hi".into(),
        metadata: serde_json::json!({}),
        content: None,
    };
    // child1: 2 messages, child2: 1, and one on the parent (not counted for a child).
    store.post_message(msg(child1.id)).await.expect("m1");
    store.post_message(msg(child1.id)).await.expect("m2");
    store.post_message(msg(child2.id)).await.expect("m3");
    store.post_message(msg(parent.id)).await.expect("m4");

    let summaries = store
        .child_thread_summaries(parent.id)
        .await
        .expect("summaries");
    assert_eq!(summaries.len(), 2, "two children, oldest first");
    assert_eq!(summaries[0].thread.id, child1.id);
    assert_eq!(summaries[0].message_count, 2);
    assert_eq!(summaries[1].thread.id, child2.id);
    assert_eq!(summaries[1].message_count, 1);

    // A thread with no children has no summaries.
    assert!(store
        .child_thread_summaries(child2.id)
        .await
        .expect("leaf")
        .is_empty());
}

#[tokio::test]
async fn child_thread_summaries_count_messages_per_child_sqlite() {
    let store = sqlite().await;
    run_child_suite(&store).await;
}

#[tokio::test]
async fn child_thread_summaries_count_messages_per_child_postgres() {
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
    run_child_suite(&store).await;
}

//! Per-workspace usage counts (Cluster 188): scoped to the workspace, exclude
//! tombstoned rows.

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    EditMessage, MemberKind, NewChannel, NewMember, NewMessage, NewThread, NewWorkspace,
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

async fn run_usage_suite(store: &dyn Store) {
    // Workspace under test: 1 member, 1 channel, 1 thread, 2 messages.
    let ws = store
        .create_workspace(NewWorkspace { name: "u1".into() })
        .await
        .expect("ws");
    let alice = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "alice".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .expect("m");
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
            title: None,
        })
        .await
        .expect("th");
    let new_msg = |body: &str| NewMessage {
        thread_id: thread.id,
        author_id: alice.id,
        body: body.into(),
        metadata: serde_json::json!({}),
        content: None,
    };
    let m1 = store.post_message(new_msg("one")).await.expect("m1");
    store.post_message(new_msg("two")).await.expect("m2");

    // A second workspace with its own content must not leak into ws's counts.
    let other = store
        .create_workspace(NewWorkspace {
            name: "other".into(),
        })
        .await
        .expect("ws2");
    let ch2 = store
        .create_channel(NewChannel {
            workspace_id: other.id,
            name: "c2".into(),
            topic: None,
            private: false,
        })
        .await
        .expect("ch2");
    let th2 = store
        .create_thread(NewThread {
            channel_id: ch2.id,
            parent_thread_id: None,
            title: None,
        })
        .await
        .expect("th2");
    let om = store
        .create_member(NewMember {
            workspace_id: other.id,
            handle: "bob".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .expect("m2");
    store
        .post_message(NewMessage {
            thread_id: th2.id,
            author_id: om.id,
            body: "elsewhere".into(),
            metadata: serde_json::json!({}),
            content: None,
        })
        .await
        .expect("om1");

    let u = store.workspace_usage(ws.id).await.expect("usage");
    assert_eq!(u.workspace_id, ws.id);
    assert_eq!(u.members, 1, "members scoped to workspace");
    assert_eq!(u.channels, 1, "channels scoped to workspace");
    assert_eq!(u.threads, 1);
    assert_eq!(u.messages, 2);

    // Tombstoning a message drops it from the count.
    store
        .edit_message(
            m1.id,
            alice.id,
            EditMessage {
                body: m1.body.clone(),
                metadata: m1.metadata.clone(),
                content: m1.content.clone(),
            },
        )
        .await
        .ok();
    store.tombstone_message(m1.id).await.expect("tombstone");
    let u2 = store.workspace_usage(ws.id).await.expect("usage2");
    assert_eq!(u2.messages, 1, "tombstoned message excluded");
}

#[tokio::test]
async fn workspace_usage_counts_are_scoped_and_exclude_tombstones_sqlite() {
    let store = sqlite().await;
    run_usage_suite(&store).await;
}

#[tokio::test]
async fn workspace_usage_counts_are_scoped_and_exclude_tombstones_postgres() {
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
    run_usage_suite(&store).await;
}

//! Spawn-budget store (Cluster 376, Wave 2 #23): the per-workspace caps + the
//! spawn-time counts the gate reads — children, nesting depth, recorded tool-use.
//! Both backends. (Enforcement is Cluster 376.2+.)

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    ContentBlock, MemberKind, NewChannel, NewMember, NewMessage, NewThread, NewWorkspace,
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
    let mk_thread = |parent: Option<maidan_types::ThreadId>| {
        let channel_id = channel.id;
        async move {
            store
                .create_thread(NewThread {
                    channel_id,
                    parent_thread_id: parent,
                    title: Some("t".into()),
                })
                .await
                .expect("thread")
        }
    };

    // --- set / get / clear the budget ---
    assert!(store.get_spawn_budget(ws.id).await.unwrap().is_none());
    let b = store
        .set_spawn_budget(ws.id, Some(8), Some(3), Some(5))
        .await
        .unwrap()
        .expect("some budget");
    assert_eq!(b.max_children, Some(8));
    assert_eq!(b.max_depth, Some(3));
    assert_eq!(b.max_tools, Some(5));
    assert_eq!(
        store
            .get_spawn_budget(ws.id)
            .await
            .unwrap()
            .unwrap()
            .max_children,
        Some(8)
    );
    // A partial update (only depth) upserts.
    let b2 = store
        .set_spawn_budget(ws.id, None, Some(2), None)
        .await
        .unwrap()
        .expect("some");
    assert_eq!(b2.max_depth, Some(2));
    assert_eq!(b2.max_children, None);
    // All-None clears the row.
    assert!(store
        .set_spawn_budget(ws.id, None, None, None)
        .await
        .unwrap()
        .is_none());
    assert!(store.get_spawn_budget(ws.id).await.unwrap().is_none());

    // --- children + depth ---
    let root = mk_thread(None).await;
    assert_eq!(
        store.thread_depth(root.id).await.unwrap(),
        1,
        "root depth 1"
    );
    assert_eq!(store.count_active_children(root.id).await.unwrap(), 0);
    let child_a = mk_thread(Some(root.id)).await;
    let _child_b = mk_thread(Some(root.id)).await;
    assert_eq!(
        store.count_active_children(root.id).await.unwrap(),
        2,
        "two direct children"
    );
    assert_eq!(
        store.thread_depth(child_a.id).await.unwrap(),
        2,
        "child depth 2"
    );
    let grandchild = mk_thread(Some(child_a.id)).await;
    assert_eq!(
        store.thread_depth(grandchild.id).await.unwrap(),
        3,
        "grandchild depth 3"
    );
    // A grandchild is not a direct child of root.
    assert_eq!(store.count_active_children(root.id).await.unwrap(), 2);

    // --- tool-use count over message content ---
    assert_eq!(store.count_thread_tool_uses(root.id).await.unwrap(), 0);
    store
        .post_message(NewMessage {
            thread_id: root.id,
            author_id: member.id,
            body: "work".into(),
            metadata: serde_json::json!({}),
            content: Some(vec![
                ContentBlock::ToolUse {
                    id: "t1".into(),
                    name: "search".into(),
                    input: serde_json::json!({}),
                },
                ContentBlock::Text { text: "hi".into() },
                ContentBlock::ToolUse {
                    id: "t2".into(),
                    name: "fetch".into(),
                    input: serde_json::json!({}),
                },
            ]),
        })
        .await
        .expect("post");
    assert_eq!(
        store.count_thread_tool_uses(root.id).await.unwrap(),
        2,
        "two tool_use blocks in one message"
    );
    // A message with no content (body only) adds nothing.
    store
        .post_message(NewMessage {
            thread_id: root.id,
            author_id: member.id,
            body: "plain".into(),
            metadata: serde_json::json!({}),
            content: None,
        })
        .await
        .expect("post2");
    store
        .post_message(NewMessage {
            thread_id: root.id,
            author_id: member.id,
            body: "more".into(),
            metadata: serde_json::json!({}),
            content: Some(vec![ContentBlock::ToolUse {
                id: "t3".into(),
                name: "run".into(),
                input: serde_json::json!({}),
            }]),
        })
        .await
        .expect("post3");
    assert_eq!(
        store.count_thread_tool_uses(root.id).await.unwrap(),
        3,
        "three tool_use blocks across messages; null-content message ignored"
    );
}

#[tokio::test]
async fn spawn_budget_caps_children_depth_and_tool_count_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn spawn_budget_caps_children_depth_and_tool_count_postgres() {
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

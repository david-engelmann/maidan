//! Spawn-budget enforcement (Cluster 376.2, Wave 2 #23): creating a child thread
//! is refused once the parent holds `max_children`, or once nesting would exceed
//! `max_depth` — a `Conflict` (SpawnRejected). Root threads + no-budget
//! workspaces are unrestricted. Both backends, via the FSM create path.

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberKind, NewChannel, NewMember, NewThread, NewWorkspace, ThreadId};
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
    let _member = store
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
    let mk = |parent: Option<ThreadId>| {
        let channel_id = channel.id;
        async move {
            store
                .create_thread(NewThread {
                    channel_id,
                    parent_thread_id: parent,
                    title: Some("t".into()),
                })
                .await
        }
    };

    // No budget → unlimited (a deep, wide tree is fine).
    let root0 = mk(None).await.expect("root0");
    for _ in 0..5 {
        mk(Some(root0.id)).await.expect("child under no budget");
    }

    // --- max_children ---
    store
        .set_spawn_budget(ws.id, Some(2), None, None)
        .await
        .unwrap();
    let root = mk(None).await.expect("root");
    mk(Some(root.id)).await.expect("child 1");
    mk(Some(root.id)).await.expect("child 2");
    let denied = mk(Some(root.id)).await;
    assert!(
        matches!(denied, Err(StoreError::Conflict(ref m)) if m.contains("spawn budget") && m.contains("child")),
        "the 3rd child must be refused, got {denied:?}"
    );
    // A root thread is never a child, so it's never blocked by max_children.
    mk(None).await.expect("another root is fine");

    // --- max_depth ---
    store
        .set_spawn_budget(ws.id, None, Some(2), None)
        .await
        .unwrap();
    let r = mk(None).await.expect("depth root (1)");
    let c = mk(Some(r.id)).await.expect("depth child (2)");
    let grand = mk(Some(c.id)).await;
    assert!(
        matches!(grand, Err(StoreError::Conflict(ref m)) if m.contains("depth")),
        "a depth-3 grandchild must be refused (max_depth 2), got {grand:?}"
    );

    // Clearing the budget re-opens spawning.
    store
        .set_spawn_budget(ws.id, None, None, None)
        .await
        .unwrap();
    mk(Some(c.id))
        .await
        .expect("grandchild allowed after clear");
}

#[tokio::test]
async fn spawn_budget_refuses_over_children_and_depth_sqlite() {
    let store = sqlite().await;
    run_suite(&store).await;
}

#[tokio::test]
async fn spawn_budget_refuses_over_children_and_depth_postgres() {
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

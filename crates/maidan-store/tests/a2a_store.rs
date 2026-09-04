//! A2A push config and task persistence (Cluster 72).

use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{NewWorkspace, WorkspaceId};
use sqlx::sqlite::SqlitePoolOptions;

#[tokio::test]
async fn a2a_push_config_and_task_persist_in_sqlite() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store = SqliteStore::new(pool);

    let ws = store
        .create_workspace(NewWorkspace { name: "a2a".into() })
        .await
        .unwrap();

    store
        .upsert_a2a_push_config(ws.id, "https://hook.example/push")
        .await
        .unwrap();
    let url = store.get_a2a_push_config(ws.id).await.unwrap();
    assert_eq!(url.as_deref(), Some("https://hook.example/push"));

    store
        .upsert_a2a_push_config(ws.id, "https://hook.example/v2")
        .await
        .unwrap();
    let url = store.get_a2a_push_config(ws.id).await.unwrap();
    assert_eq!(url.as_deref(), Some("https://hook.example/v2"));

    let task = serde_json::json!({
        "id": "task-1",
        "status": { "state": "TASK_STATE_WORKING" }
    });
    store
        .upsert_a2a_task(ws.id, "task-1", task.clone())
        .await
        .unwrap();
    let loaded = store.get_a2a_task("task-1").await.unwrap().unwrap();
    assert_eq!(loaded["id"], "task-1");
    assert_eq!(
        store.get_a2a_task_workspace("task-1").await.unwrap(),
        Some(ws.id)
    );
    assert_eq!(
        store
            .get_a2a_push_config(WorkspaceId(uuid::Uuid::new_v4()))
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn list_a2a_tasks_returns_workspace_tasks_within_limit() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store = SqliteStore::new(pool);

    let ws_a = store
        .create_workspace(NewWorkspace { name: "a".into() })
        .await
        .unwrap();
    let ws_b = store
        .create_workspace(NewWorkspace { name: "b".into() })
        .await
        .unwrap();

    for i in 0..3 {
        let id = format!("a-{i}");
        store
            .upsert_a2a_task(
                ws_a.id,
                &id,
                serde_json::json!({"id": id, "status": {"state": "TASK_STATE_WORKING"}}),
            )
            .await
            .unwrap();
    }
    store
        .upsert_a2a_task(
            ws_b.id,
            "b-0",
            serde_json::json!({"id": "b-0", "status": {"state": "TASK_STATE_WORKING"}}),
        )
        .await
        .unwrap();

    // Workspace-scoped: ws_a sees only its 3 tasks, not ws_b's.
    let a_tasks = store.list_a2a_tasks(ws_a.id, 50, None).await.unwrap();
    assert_eq!(a_tasks.len(), 3);
    assert!(a_tasks
        .iter()
        .all(|t| t["id"].as_str().unwrap().starts_with("a-")));

    // Limit is honored.
    let limited = store.list_a2a_tasks(ws_a.id, 2, None).await.unwrap();
    assert_eq!(limited.len(), 2);

    // Empty workspace lists nothing.
    let empty = store
        .list_a2a_tasks(WorkspaceId(uuid::Uuid::new_v4()), 50, None)
        .await
        .unwrap();
    assert!(empty.is_empty());
}

#[tokio::test]
async fn a2a_task_push_configs_crud() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store = SqliteStore::new(pool);

    store
        .create_a2a_task_push_config("task-1", "cfg-a", "https://a.example")
        .await
        .unwrap();
    store
        .create_a2a_task_push_config("task-1", "cfg-b", "https://b.example")
        .await
        .unwrap();
    store
        .create_a2a_task_push_config("task-2", "cfg-c", "https://c.example")
        .await
        .unwrap();

    // get: present + absent.
    assert_eq!(
        store
            .get_a2a_task_push_config("task-1", "cfg-a")
            .await
            .unwrap()
            .as_deref(),
        Some("https://a.example")
    );
    assert_eq!(
        store
            .get_a2a_task_push_config("task-1", "missing")
            .await
            .unwrap(),
        None
    );

    // list is task-scoped.
    assert_eq!(
        store
            .list_a2a_task_push_configs("task-1")
            .await
            .unwrap()
            .len(),
        2
    );
    assert_eq!(
        store
            .list_a2a_task_push_configs("task-2")
            .await
            .unwrap()
            .len(),
        1
    );

    // create with an existing (task, config) upserts the url, not a new row.
    store
        .create_a2a_task_push_config("task-1", "cfg-a", "https://a2.example")
        .await
        .unwrap();
    assert_eq!(
        store
            .get_a2a_task_push_config("task-1", "cfg-a")
            .await
            .unwrap()
            .as_deref(),
        Some("https://a2.example")
    );
    assert_eq!(
        store
            .list_a2a_task_push_configs("task-1")
            .await
            .unwrap()
            .len(),
        2
    );

    // delete: removes, then reports no-op.
    assert!(store
        .delete_a2a_task_push_config("task-1", "cfg-a")
        .await
        .unwrap());
    assert!(!store
        .delete_a2a_task_push_config("task-1", "cfg-a")
        .await
        .unwrap());
    assert_eq!(
        store
            .list_a2a_task_push_configs("task-1")
            .await
            .unwrap()
            .len(),
        1
    );
}

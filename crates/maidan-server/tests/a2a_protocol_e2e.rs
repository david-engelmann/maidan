//! A2A protocol JSON-RPC: SendMessage posts to a thread; GetTask returns the task.

use std::{sync::Arc, time::Duration};

use maidan_a2a::{A2aClient, SendMessageRequest};
use maidan_artifacts::LocalFsStore;
use maidan_bus::InMemoryBus;
use maidan_server::{router, AppState};
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;

#[tokio::test]
async fn a2a_send_message_posts_to_thread_and_get_task_round_trips() {
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(InMemoryBus::with_capacity(64));

    let app = router(AppState::for_tests(store, artifacts, bus, search));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let _server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let base = format!("http://{addr}");

    let ws: serde_json::Value = client
        .post(format!("{base}/workspaces"))
        .json(&json!({"name": "a2a-ws"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let workspace_id = ws["id"].as_str().unwrap();

    let member: serde_json::Value = client
        .post(format!("{base}/workspaces/{workspace_id}/members"))
        .json(&json!({"handle": "agent", "kind": "agent"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let author_id = member["id"].as_str().unwrap();

    let ch: serde_json::Value = client
        .post(format!("{base}/workspaces/{workspace_id}/channels"))
        .json(&json!({"name": "general"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let channel_id = ch["id"].as_str().unwrap();

    let th: serde_json::Value = client
        .post(format!("{base}/channels/{channel_id}/threads"))
        .json(&json!({"title": "a2a"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let thread_id = th["id"].as_str().unwrap();

    let a2a = A2aClient::new(&base).unwrap();
    let result = a2a
        .send_message(
            serde_json::from_value::<SendMessageRequest>(json!({
                "message": {
                    "role": "user",
                    "parts": [{ "type": "text", "text": "via a2a" }]
                },
                "metadata": {
                    "maidan": { "threadId": thread_id, "authorId": author_id }
                }
            }))
            .unwrap(),
        )
        .await
        .unwrap();
    let task_id = result["task"]["id"].as_str().expect("task id");
    assert_eq!(
        result["task"]["status"]["state"].as_str(),
        Some("TASK_STATE_COMPLETED")
    );

    let task = a2a.get_task(task_id).await.unwrap();
    assert_eq!(task.id, task_id);

    let listed: Vec<serde_json::Value> = client
        .get(format!("{base}/threads/{thread_id}/messages"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        listed.iter().any(|m| m["body"].as_str() == Some("via a2a")),
        "message body not found"
    );
}

//! A2A protocol JSON-RPC: SendMessage posts to a thread; GetTask returns the task.

use std::{sync::Arc, time::Duration};

use futures::StreamExt;
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

#[tokio::test]
async fn a2a_send_message_preserves_parts_as_structured_content() {
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
    let client = reqwest::Client::new();
    let base = format!("http://{addr}");

    let ws: serde_json::Value = client
        .post(format!("{base}/workspaces"))
        .json(&json!({"name": "a2a-content"}))
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
    a2a.send_message(
        serde_json::from_value::<SendMessageRequest>(json!({
            "message": {
                "role": "user",
                "parts": [
                    { "type": "text", "text": "part one" },
                    { "type": "text", "text": "part two" }
                ]
            },
            "metadata": { "maidan": { "threadId": thread_id, "authorId": author_id } }
        }))
        .unwrap(),
    )
    .await
    .unwrap();

    let listed: Vec<serde_json::Value> = client
        .get(format!("{base}/threads/{thread_id}/messages"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let msg = listed
        .iter()
        .find(|m| m["body"].as_str() == Some("part one\npart two"))
        .expect("a2a message with joined body");
    // The parts are preserved as structured content blocks (Cluster 194).
    let content = msg["content"].as_array().expect("content array");
    assert_eq!(content.len(), 2);
    assert_eq!(content[0], json!({"type": "text", "text": "part one"}));
    assert_eq!(content[1], json!({"type": "text", "text": "part two"}));
}

#[tokio::test]
async fn a2a_send_streaming_message_returns_sse_task_updates() {
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
        .json(&json!({"name": "a2a-stream-ws"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let workspace_id = ws["id"].as_str().unwrap();

    let member: serde_json::Value = client
        .post(format!("{base}/workspaces/{workspace_id}/members"))
        .json(&json!({"handle": "stream-agent", "kind": "agent"}))
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
        .json(&json!({"title": "stream"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let thread_id = th["id"].as_str().unwrap();

    let a2a = A2aClient::new(&base).unwrap();
    let events = a2a
        .send_streaming_message(
            serde_json::from_value::<SendMessageRequest>(json!({
                "message": {
                    "role": "user",
                    "parts": [{ "type": "text", "text": "streamed" }]
                },
                "metadata": {
                    "maidan": { "threadId": thread_id, "authorId": author_id }
                }
            }))
            .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        events.len() >= 2,
        "expected SSE frames, got {}",
        events.len()
    );
    let first = events[0].result.as_ref().expect("first result");
    assert_eq!(
        first["task"]["status"]["state"].as_str(),
        Some("TASK_STATE_WORKING")
    );
    let second = events[1].result.as_ref().expect("second result");
    assert_eq!(
        second["statusUpdate"]["status"]["state"].as_str(),
        Some("TASK_STATE_COMPLETED")
    );
    assert_eq!(second["statusUpdate"]["final"].as_bool(), Some(true));
}

#[tokio::test]
async fn a2a_get_task_loads_from_store_after_send_message() {
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
        .json(&json!({"name": "a2a-persist"}))
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
        .json(&json!({}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let thread_id = th["id"].as_str().unwrap();

    let a2a = A2aClient::new(&base).unwrap();
    let send_resp = a2a
        .send_message(
            serde_json::from_value::<SendMessageRequest>(json!({
                "message": {
                    "role": "user",
                    "parts": [{ "type": "text", "text": "persisted" }]
                },
                "metadata": {
                    "maidan": { "threadId": thread_id, "authorId": author_id }
                }
            }))
            .unwrap(),
        )
        .await
        .unwrap();
    let task_id = send_resp["task"]["id"].as_str().expect("task id");

    let task = a2a.get_task(task_id).await.unwrap();
    assert_eq!(task.id, task_id);
    assert_eq!(task.status.state, "TASK_STATE_COMPLETED");
}

#[tokio::test]
async fn a2a_subscribe_to_task_rejects_terminal_task() {
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

    let ws = store
        .create_workspace(maidan_types::NewWorkspace { name: "sub".into() })
        .await
        .unwrap();
    let task = serde_json::json!({
        "id": "done-task",
        "status": { "state": "TASK_STATE_COMPLETED" }
    });
    store
        .upsert_a2a_task(ws.id, "done-task", task)
        .await
        .unwrap();

    let app = router(AppState::for_tests(store, artifacts, bus, search));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let _server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let client = reqwest::Client::new();
    let resp: serde_json::Value = client
        .post(format!("http://{addr}/a2a/v1/rpc"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "SubscribeToTask",
            "params": { "id": "done-task" }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(resp["error"]["code"], -32005);
}

#[tokio::test]
async fn a2a_subscribe_to_task_streams_working_task() {
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

    let ws = store
        .create_workspace(maidan_types::NewWorkspace {
            name: "sub2".into(),
        })
        .await
        .unwrap();
    store
        .upsert_a2a_task(
            ws.id,
            "work-task",
            serde_json::json!({
                "id": "work-task",
                "status": { "state": "TASK_STATE_WORKING" }
            }),
        )
        .await
        .unwrap();

    let app = router(AppState::for_tests(store, artifacts, bus, search));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let _server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://{addr}/a2a/v1/rpc"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "SubscribeToTask",
            "params": { "id": "work-task" }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let body = resp.text().await.unwrap();
    assert!(body.contains("TASK_STATE_WORKING"));
    assert!(body.contains("work-task"));
}

#[tokio::test]
async fn a2a_tasks_cancel_marks_working_task_canceled() {
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

    let ws = store
        .create_workspace(maidan_types::NewWorkspace {
            name: "cancel-ws".into(),
        })
        .await
        .unwrap();
    store
        .upsert_a2a_task(
            ws.id,
            "cancel-me",
            serde_json::json!({
                "id": "cancel-me",
                "status": { "state": "TASK_STATE_WORKING" }
            }),
        )
        .await
        .unwrap();

    let app = router(AppState::for_tests(store, artifacts, bus, search));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let _server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let client = reqwest::Client::new();
    let resp: serde_json::Value = client
        .post(format!("http://{addr}/a2a/v1/rpc"))
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tasks/cancel",
            "params": { "id": "cancel-me" }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(resp["result"]["status"]["state"], "TASK_STATE_CANCELED");

    let resp2: serde_json::Value = client
        .post(format!("http://{addr}/a2a/v1/rpc"))
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "SubscribeToTask",
            "params": { "id": "cancel-me" }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(resp2["error"]["code"], -32005);
}

#[tokio::test]
async fn a2a_subscribe_to_task_emits_progress_when_task_becomes_terminal() {
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

    let ws = store
        .create_workspace(maidan_types::NewWorkspace {
            name: "progress-ws".into(),
        })
        .await
        .unwrap();
    store
        .upsert_a2a_task(
            ws.id,
            "progress-task",
            serde_json::json!({
                "id": "progress-task",
                "status": { "state": "TASK_STATE_WORKING" }
            }),
        )
        .await
        .unwrap();

    let store_bg = store.clone();
    let ws_id = ws.id;
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(250)).await;
        store_bg
            .upsert_a2a_task(
                ws_id,
                "progress-task",
                serde_json::json!({
                    "id": "progress-task",
                    "status": { "state": "TASK_STATE_COMPLETED" }
                }),
            )
            .await
            .unwrap();
    });

    let app = router(AppState::for_tests(store, artifacts, bus, search));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let _server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    let resp = client
        .post(format!("http://{addr}/a2a/v1/rpc"))
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "SubscribeToTask",
            "params": { "id": "progress-task" }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    let mut buf = String::new();
    let mut stream = resp.bytes_stream();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    while tokio::time::Instant::now() < deadline {
        let Some(chunk) = stream.next().await else {
            break;
        };
        buf.push_str(&String::from_utf8_lossy(&chunk.unwrap()));
        if buf.contains("TASK_STATE_COMPLETED") && buf.contains("statusUpdate") {
            break;
        }
    }
    assert!(
        buf.contains("TASK_STATE_WORKING"),
        "expected initial task frame"
    );
    assert!(
        buf.contains("statusUpdate"),
        "expected progress frame, got: {buf}"
    );
    assert!(buf.contains("TASK_STATE_COMPLETED"));
}

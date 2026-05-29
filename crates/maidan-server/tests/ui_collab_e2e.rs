//! UI v3 collaboration: ui/api reads for threads, messages, faceted search.

use std::{sync::Arc, time::Duration};

use maidan_artifacts::LocalFsStore;
use maidan_bus::InMemoryBus;
use maidan_server::{router, AppState};
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use reqwest::StatusCode;
use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;

async fn spawn() -> (
    std::net::SocketAddr,
    reqwest::Client,
    tokio::task::JoinHandle<()>,
) {
    let pool = SqlitePoolOptions::new()
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
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    (addr, client, server)
}

#[tokio::test]
async fn ui_v3_collab_shell_and_session_api_reads() {
    let (addr, client, server) = spawn().await;
    let base = format!("http://{addr}");

    let html = client
        .get(format!("{base}/ui/"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(html.contains(r#"data-ui-version="3""#));
    assert!(html.contains(r#"id="thread-list""#));
    assert!(html.contains(r#"id="collab-panel""#));
    assert!(html.contains("create-channel"));
    assert!(html.contains("upload-artifact"));

    let ws: serde_json::Value = client
        .post(format!("{base}/workspaces"))
        .json(&json!({"name": "ui-collab"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let workspace_id = ws["id"].as_str().unwrap();

    let alice: serde_json::Value = client
        .post(format!("{base}/workspaces/{workspace_id}/members"))
        .json(&json!({"handle": "alice", "kind": "human"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let alice_id = alice["id"].as_str().unwrap();

    let channel: serde_json::Value = client
        .post(format!("{base}/workspaces/{workspace_id}/channels"))
        .json(&json!({"name": "general"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let channel_id = channel["id"].as_str().unwrap();

    let thread: serde_json::Value = client
        .post(format!("{base}/channels/{channel_id}/threads"))
        .json(&json!({"title": "collab-thread"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let thread_id = thread["id"].as_str().unwrap();

    let msg: serde_json::Value = client
        .post(format!("{base}/threads/{thread_id}/messages"))
        .json(&json!({"author_id": alice_id, "body": "faceted search needle collab-v44"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(msg["body"], "faceted search needle collab-v44");

    let threads: Vec<serde_json::Value> = client
        .get(format!("{base}/ui/api/channels/{channel_id}/threads"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(threads.len(), 1);
    assert_eq!(threads[0]["title"], "collab-thread");

    let messages: Vec<serde_json::Value> = client
        .get(format!(
            "{base}/ui/api/threads/{thread_id}/messages?limit=10"
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(messages.len(), 1);

    let hits: Vec<serde_json::Value> = client
        .get(format!(
            "{base}/ui/api/workspaces/{workspace_id}/search?q=needle&mode=lexical&channel={channel_id}&limit=10"
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        !hits.is_empty(),
        "expected lexical hit for channel-filtered search"
    );

    let artifact = client
        .post(format!(
            "{base}/artifacts?kind=attachment&mime_type=text/plain"
        ))
        .body(b"ui-collab artifact bytes".to_vec())
        .send()
        .await
        .unwrap();
    assert_eq!(artifact.status(), StatusCode::CREATED);

    server.abort();
}

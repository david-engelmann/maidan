//! End-to-end: HTTP server publishes to the bus; the indexer task
//! sees `MessagePosted` within the 500 ms SLO.

use std::{sync::Arc, time::Duration};

use maidan_artifacts::LocalFsStore;
use maidan_bus::{EventBus, InMemoryBus};
use maidan_search::{Indexer, LoggingHandler, SqliteSearch};
use maidan_server::{router, AppState};
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::EventKind;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

#[tokio::test]
async fn http_post_drives_indexer() {
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
    let search: Arc<dyn maidan_search::Search> = Arc::new(SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus: Arc<dyn EventBus> = Arc::new(InMemoryBus::with_capacity(256));

    let handler = Arc::new(LoggingHandler::default());
    let indexer = Indexer::new(bus.clone(), handler.clone()).spawn();
    tokio::time::sleep(Duration::from_millis(20)).await;

    let app = router(AppState::for_tests(store, artifacts, bus, search));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let base = format!("http://{addr}");

    let ws: Value = client
        .post(format!("{base}/workspaces"))
        .json(&json!({"name": "idx"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let workspace_id = ws["id"].as_str().unwrap().to_string();
    let alice: Value = client
        .post(format!("{base}/workspaces/{workspace_id}/members"))
        .json(&json!({"handle": "alice", "kind": "human"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let alice_id = alice["id"].as_str().unwrap().to_string();
    let ch: Value = client
        .post(format!("{base}/workspaces/{workspace_id}/channels"))
        .json(&json!({"name": "general"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let channel_id = ch["id"].as_str().unwrap().to_string();
    let th: Value = client
        .post(format!("{base}/channels/{channel_id}/threads"))
        .json(&json!({"title": "t"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let thread_id = th["id"].as_str().unwrap().to_string();
    let _: Value = client
        .post(format!("{base}/threads/{thread_id}/messages"))
        .json(&json!({"author_id": alice_id, "body": "hello"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let observed = handler
        .wait_for(Duration::from_millis(500), |log| {
            log.contains(&EventKind::MessagePosted)
        })
        .await
        .expect("indexer did not observe MessagePosted within 500 ms");

    assert!(observed.contains(&EventKind::MessagePosted));

    server.abort();
    indexer.shutdown().await;
}

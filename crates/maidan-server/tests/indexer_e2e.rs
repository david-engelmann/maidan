//! End-to-end: HTTP server publishes to the bus; the indexer task
//! sees `MessagePosted` within the 500 ms SLO.

use std::{sync::Arc, time::Duration};

use maidan_artifacts::LocalFsStore;
use maidan_bus::{EventBus, InMemoryBus};
use maidan_search::{Indexer, LoggingHandler, SqliteSearch};
use maidan_server::{router, test_support, AppState};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{EventKind, MemberKind};
use serde_json::{json, Value};

#[tokio::test]
async fn http_post_drives_indexer() {
    let pool = maidan_search::sqlite_pool_options()
        .max_connections(4)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
    let tenant = test_support::tenant(store.as_ref(), "idx").await;
    let alice = tenant
        .member(store.as_ref(), "alice", MemberKind::Human)
        .await;
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
    let client = tenant.client();
    let base = format!("http://{addr}");
    let workspace_id = tenant.workspace_id.0;
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
    let _: Value = alice
        .client()
        .post(format!("{base}/threads/{thread_id}/messages"))
        .json(&json!({"body": "hello"}))
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

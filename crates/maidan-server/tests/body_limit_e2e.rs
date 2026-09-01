//! Request body-size cap (Cluster 183): a body over `MAIDAN_MAX_BODY_BYTES` is
//! rejected with 413 before the handler buffers it.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use reqwest::StatusCode;
use sqlx::sqlite::SqlitePoolOptions;

async fn spawn() -> (SocketAddr, reqwest::Client, tokio::task::JoinHandle<()>) {
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
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    // auth DISABLED so the request reaches the body extractor (auth would 401
    // first). `router` reads MAIDAN_MAX_BODY_BYTES at build time.
    let state = AppState::new(
        store,
        artifacts,
        bus,
        search,
        Arc::new(maidan_search::HashV1Provider),
        true,
        true,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, reqwest::Client::new(), server)
}

#[tokio::test]
async fn oversized_request_body_is_rejected() {
    // Set the cap before the router is built (it reads env once at build).
    std::env::set_var("MAIDAN_MAX_BODY_BYTES", "1024");
    let (addr, client, server) = spawn().await;
    let base = format!("http://{addr}");

    // A ~4 KiB body to a JSON endpoint exceeds the 1 KiB cap → 413.
    let big = "x".repeat(4096);
    let over = client
        .post(format!("{base}/workspaces"))
        .header("content-type", "application/json")
        .body(serde_json::json!({ "name": big }).to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(over.status(), StatusCode::PAYLOAD_TOO_LARGE);

    // A small well-formed body is accepted.
    let ok = client
        .post(format!("{base}/workspaces"))
        .json(&serde_json::json!({ "name": "small" }))
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status(), StatusCode::CREATED);

    std::env::remove_var("MAIDAN_MAX_BODY_BYTES");
    server.abort();
}

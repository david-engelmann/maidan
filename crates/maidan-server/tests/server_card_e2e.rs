//! The MCP server card over HTTP: a catalog reads it before it holds any
//! credential, so it answers unauthenticated on an instance with nothing
//! configured, and what it serves is the live catalog, not a copy.

use std::sync::{atomic::AtomicI64, Arc};

use maidan_artifacts::LocalFsStore;
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use reqwest::StatusCode;
use serde_json::Value;
use sqlx::sqlite::SqlitePoolOptions;

async fn spawn() -> String {
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(
        store,
        Arc::new(LocalFsStore::new(dir.path())),
        Arc::new(maidan_bus::InMemoryBus::new()),
        search,
        Arc::new(maidan_search::HashV1Provider),
        false,
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

#[tokio::test]
async fn the_server_card_answers_unauthenticated_with_the_live_catalog() {
    let base = spawn().await;
    let response = reqwest::Client::new()
        .get(format!("{base}/.well-known/mcp/server-card.json"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let card: Value = response.json().await.unwrap();
    assert_eq!(card["serverInfo"]["name"], "maidan");
    assert_eq!(card["authentication"]["required"], true);
    assert_eq!(
        card["tools"],
        Value::Array(maidan_mcp::tools::catalog()),
        "the served card's tools are the catalog tools/list returns"
    );
}

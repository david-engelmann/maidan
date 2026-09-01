//! Bootstrap HTTP routes are absent when the `bootstrap` feature is disabled.
#![cfg(not(feature = "bootstrap"))]

use std::{net::SocketAddr, sync::Arc};

use maidan_artifacts::LocalFsStore;
use maidan_server::{router, AppState};
use maidan_store::{prelude::*, run_sqlite_migrations};
use reqwest::StatusCode;
use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;

async fn spawn() -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("foreign_keys");
    run_sqlite_migrations(&pool).await.expect("migrate");

    let store = Arc::new(SqliteStore::new(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let artifacts = Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path()));
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    maidan_server::metrics::init();
    let app = router(AppState::for_tests(store, artifacts, bus, search));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, server)
}

#[tokio::test]
async fn bootstrap_routes_are_not_registered_without_feature() {
    let (addr, server) = spawn().await;
    let client = reqwest::Client::new();

    let resp = client
        .post(format!("http://{addr}/workspaces"))
        .json(&json!({ "name": "nope" }))
        .send()
        .await
        .expect("post");
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    server.abort();
}

#[tokio::test]
async fn openapi_omits_bootstrap_paths_without_feature() {
    let (addr, server) = spawn().await;
    let body = reqwest::Client::new()
        .get(format!("http://{addr}/openapi.json"))
        .send()
        .await
        .expect("openapi")
        .text()
        .await
        .expect("body");

    let doc: serde_json::Value = serde_json::from_str(&body).expect("json");
    let paths = doc["paths"].as_object().expect("paths object");
    assert!(!paths.contains_key("/workspaces"));
    let tags = doc["tags"]
        .as_array()
        .map(|t| {
            t.iter()
                .filter_map(|tag| tag["name"].as_str())
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    assert!(!tags.iter().any(|name| *name == "bootstrap"));

    server.abort();
}

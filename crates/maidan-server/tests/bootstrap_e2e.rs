//! Bootstrap route gating when bearer auth is enabled.
#![cfg(feature = "bootstrap")]

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use maidan_artifacts::LocalFsStore;
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use reqwest::StatusCode;
use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;

struct Harness {
    addr: SocketAddr,
    server: tokio::task::JoinHandle<()>,
    client: reqwest::Client,
    _dir: tempfile::TempDir,
}

impl Harness {
    fn base(&self) -> String {
        format!("http://{}", self.addr)
    }

    async fn shutdown(self) {
        self.server.abort();
    }
}

async fn spawn(bootstrap_enabled: bool) -> Harness {
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
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    let app = router(AppState::new(
        store,
        artifacts,
        bus,
        search,
        Arc::new(maidan_search::HashV1Provider),
        false,
        bootstrap_enabled,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    Harness {
        addr,
        server,
        client,
        _dir: dir,
    }
}

#[tokio::test]
async fn bootstrap_routes_reject_when_flag_unset_and_auth_enabled() {
    let h = spawn(false).await;
    let base = h.base();
    let res = h
        .client
        .post(format!("{base}/workspaces"))
        .json(&json!({ "name": "blocked" }))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
    h.shutdown().await;
}

#[tokio::test]
async fn bootstrap_creates_workspace_and_member_when_flag_set() {
    let h = spawn(true).await;
    let base = h.base();
    let ws: serde_json::Value = h
        .client
        .post(format!("{base}/workspaces"))
        .json(&json!({ "name": "seed" }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    let wid = ws["id"].as_str().unwrap();
    let member = h
        .client
        .post(format!("{base}/workspaces/{wid}/members"))
        .json(&json!({
            "handle": "admin",
            "kind": "agent"
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(member.status(), StatusCode::CREATED);
    h.shutdown().await;
}

#[tokio::test]
async fn bootstrap_rejects_second_workspace_creation() {
    let h = spawn(true).await;
    let base = h.base();
    let first = h
        .client
        .post(format!("{base}/workspaces"))
        .json(&json!({ "name": "one" }))
        .send()
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::CREATED);
    let second = h
        .client
        .post(format!("{base}/workspaces"))
        .json(&json!({ "name": "two" }))
        .send()
        .await
        .unwrap();
    assert_eq!(second.status(), StatusCode::FORBIDDEN);
    h.shutdown().await;
}

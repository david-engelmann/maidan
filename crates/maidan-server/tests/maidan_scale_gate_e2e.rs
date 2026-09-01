//! Cluster 120: `maidan-scale-1.0` gate — the scale-out runtime surfaces and
//! the metrics gauges that the scale gate is built on respond in-process.
//!
//! Multi-replica / cross-replica behavior (102–105) is covered by the
//! `two_replica_*_e2e` tests and the `scale-out smoke` CI job; this checklist
//! asserts the single-process surfaces and the scale-specific telemetry
//! (indexer lag/queue depth from 116, indexer heartbeat) are wired.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_server::{router, subscribe_resume, AppState, FederationRuntime};
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
    let mut state = AppState::new(
        store,
        artifacts,
        bus,
        search,
        Arc::new(maidan_search::HashV1Provider),
        true,
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    state.subscribe_resume_secret = Some(Arc::from(subscribe_resume::TEST_SUBSCRIBE_RESUME_SECRET));
    maidan_server::metrics::init();
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, reqwest::Client::new(), server)
}

#[tokio::test]
async fn maidan_scale_gate_surfaces_respond() {
    let (addr, client, server) = spawn().await;
    let base = format!("http://{addr}");

    // Core operator/runtime surfaces (must not regress maidan-operator-1.0).
    for path in [
        "/health",
        "/health/ready",
        "/metrics",
        "/openapi.json",
        "/.well-known/maidan.json",
    ] {
        assert_eq!(
            client
                .get(format!("{base}{path}"))
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::OK,
            "{path}"
        );
    }

    // Scale-specific telemetry: the indexer heartbeat (health/readiness) and the
    // bounded-lag pipeline gauges (Cluster 116) the gate's perf story rests on.
    let metrics = client
        .get(format!("{base}/metrics"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    for gauge in [
        "maidan_indexer_last_event_age_seconds",
        "maidan_indexer_queue_depth",
        "maidan_indexer_queue_capacity",
    ] {
        assert!(metrics.contains(gauge), "metrics must expose {gauge}");
    }

    server.abort();
}

//! `GET /metrics` exposes Prometheus text (Track T.4).

use std::{net::SocketAddr, sync::Arc, time::Duration};

use maidan_artifacts::LocalFsStore;
use maidan_server::{router, AppState};
use maidan_store::{run_sqlite_migrations, SqliteStore};
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
async fn metrics_endpoint_returns_prometheus_text() {
    let (addr, server) = spawn().await;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("client");

    let _ = client
        .get(format!("http://{addr}/health/live"))
        .send()
        .await
        .expect("warmup");

    let body = client
        .get(format!("http://{addr}/metrics"))
        .send()
        .await
        .expect("metrics")
        .text()
        .await
        .expect("body");

    assert!(body.contains("http_server_request_total"));
    assert!(body.contains("maidan_indexer_last_event_age_seconds"));

    server.abort();
}

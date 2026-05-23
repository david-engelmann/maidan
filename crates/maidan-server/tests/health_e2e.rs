//! End-to-end test for the `/health` endpoint.
//!
//! Spins up a real Postgres testcontainer, a temp directory artifact
//! store, binds the axum router to a random localhost port, and curls
//! `/health` through `reqwest`. Verifies 200 status and the structured
//! body shape. Skips if Docker is unavailable.

use std::{net::SocketAddr, sync::Arc, time::Duration};

use maidan_artifacts::LocalFsStore;
use maidan_server::{router, AppState};
use maidan_store::{run_postgres_migrations, PostgresStore};
use sqlx::postgres::PgPoolOptions;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

async fn spawn_server() -> Option<(
    SocketAddr,
    tokio::task::JoinHandle<()>,
    tempfile::TempDir,
    testcontainers::ContainerAsync<Postgres>,
)> {
    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            eprintln!("skipping health_e2e: docker unavailable ({err})");
            return None;
        }
    };

    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");

    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .expect("connect");
    run_postgres_migrations(&pool).await.expect("migrate");

    let dir = tempfile::tempdir().expect("tempdir");
    let store = Arc::new(PostgresStore::new(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::PostgresSearch::new(pool));
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    let app = router(AppState::new(store, artifacts, bus, search));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind random port");
    let addr = listener.local_addr().expect("local addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    Some((addr, handle, dir, container))
}

#[tokio::test]
async fn health_returns_200_with_structured_body() {
    let Some((addr, handle, _dir, _container)) = spawn_server().await else {
        return;
    };

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let resp = client
        .get(format!("http://{addr}/health"))
        .send()
        .await
        .expect("send /health");
    assert!(
        resp.status().is_success(),
        "expected 200, got {}",
        resp.status()
    );

    let body: serde_json::Value = resp.json().await.expect("parse json");
    assert_eq!(body["status"], "ok");
    assert_eq!(body["db"], "ok");
    assert_eq!(body["storage"], "ok");
    assert!(body.get("version").is_some(), "expected version field");

    handle.abort();
}

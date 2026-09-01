//! End-to-end test for the `/health` endpoint.
//!
//! Spins up a real Postgres testcontainer, a temp directory artifact
//! store, binds the axum router to a random localhost port, and curls
//! `/health` through `reqwest`. Verifies 200 status and the structured
//! body shape. Skips if Docker is unavailable.

use std::{net::SocketAddr, sync::Arc, time::Duration};

use maidan_artifacts::LocalFsStore;
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_postgres_migrations};
use reqwest::StatusCode;
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
    let app = router(AppState::for_tests(store, artifacts, bus, search));

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
    assert_eq!(body["bus"], "ok");
    assert!(body.get("version").is_some(), "expected version field");

    let live = client
        .get(format!("http://{addr}/health/live"))
        .send()
        .await
        .expect("send /health/live");
    assert!(live.status().is_success());

    let resp2 = client
        .get(format!("http://{addr}/health"))
        .send()
        .await
        .expect("send /health again");
    assert!(resp2.headers().get("x-request-id").is_some());

    handle.abort();
}

#[tokio::test]
async fn health_reports_postgres_bus_ok_after_notify() {
    let Some((addr, handle, _dir, _container)) = spawn_server_with_postgres_bus().await else {
        return;
    };

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();

    tokio::time::sleep(Duration::from_millis(200)).await;

    let resp = client
        .get(format!("http://{addr}/health"))
        .send()
        .await
        .expect("send /health");
    assert!(resp.status().is_success());
    let body: serde_json::Value = resp.json().await.expect("parse json");
    assert_eq!(body["bus"], "ok");

    handle.abort();
}

#[tokio::test]
async fn health_reports_indexer_embedding_errors() {
    use maidan_search::SqliteSearch;
    use maidan_store::{run_sqlite_migrations, SqliteStore};
    use sqlx::sqlite::SqlitePoolOptions;

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

    let store = Arc::new(SqliteStore::new(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(SqliteSearch::new(pool));
    let dir = tempfile::tempdir().expect("tempdir");
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    let state = AppState::for_tests(store, artifacts, bus, search);
    *state.indexer_last_error.write().await = Some("remote provider timeout".into());
    let app = router(state);

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();

    let resp = client
        .get(format!("http://{addr}/health/ready"))
        .send()
        .await
        .expect("send /health/ready");
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    let body: serde_json::Value = resp.json().await.expect("json");
    let idx = body["indexer"]["error"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(
        idx.contains("embedding indexer error"),
        "unexpected indexer status: {idx}"
    );

    handle.abort();
}

async fn spawn_server_with_postgres_bus() -> Option<(
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
            eprintln!("skipping health_e2e postgres bus: docker unavailable ({err})");
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
    let search: Arc<dyn maidan_search::Search> =
        Arc::new(maidan_search::PostgresSearch::new(pool.clone()));
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let pg_bus = maidan_bus::PostgresBus::connect(pool)
        .await
        .expect("postgres bus");
    let bus_health = pg_bus.listener_health();
    let bus: Arc<dyn maidan_bus::EventBus> = Arc::new(pg_bus);

    let app = router(AppState::new(
        store,
        artifacts,
        bus.clone(),
        search,
        Arc::new(maidan_search::HashV1Provider),
        true,
        false,
        FederationRuntime::new(true, None),
        Arc::new(std::sync::atomic::AtomicI64::new(0)),
        Some(bus_health),
    ));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind random port");
    let addr = listener.local_addr().expect("local addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    use maidan_types::{Event, Workspace, WorkspaceId};
    let ws = Workspace {
        id: WorkspaceId(uuid::Uuid::new_v4()),
        name: "bus-health".into(),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
        tombstoned_at: None,
    };
    let event = Event::WorkspaceCreated {
        occurred_at: chrono::Utc::now(),
        workspace: ws,
    };
    maidan_bus::EventBus::publish(bus.as_ref(), maidan_types::BusEnvelope::synthetic(event))
        .await
        .expect("publish");

    Some((addr, handle, dir, container))
}

//! HTTP + SQLite outbox: publish defers in-memory bus delivery until relay runs.

use std::{net::SocketAddr, sync::Arc, time::Duration};

use futures::StreamExt;
use maidan_artifacts::LocalFsStore;
use maidan_bus::{BusItem, EventBus, InMemoryBus};
use maidan_server::{outbox_relay::OutboxRelay, router, AppState};
use maidan_store::{run_sqlite_migrations, sqlite::outbox, OutboxBackend, SqliteStore, Store};
use maidan_types::EventFilter;
use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;

async fn spawn_sqlite_outbox() -> Option<(
    SocketAddr,
    sqlx::SqlitePool,
    Arc<InMemoryBus>,
    tokio::task::JoinHandle<()>,
)> {
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
        .connect("sqlite::memory:")
        .await
        .ok()?;
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .ok()?;
    run_sqlite_migrations(&pool).await.ok()?;

    let bus = Arc::new(InMemoryBus::new());
    let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
    let search: Arc<dyn maidan_search::Search> =
        Arc::new(maidan_search::SqliteSearch::new(pool.clone()));
    let artifacts = Arc::new(LocalFsStore::new(tempfile::tempdir().ok()?.path()));

    let mut state = AppState::for_tests(store, artifacts, bus.clone(), search);
    state.outbox_relay = true;
    state.outbox_backend = Some(OutboxBackend::Sqlite(pool.clone()));

    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.ok()?;
    let addr = listener.local_addr().ok()?;
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    Some((addr, pool, bus, server))
}

#[tokio::test]
async fn sqlite_http_mutation_defers_bus_until_outbox_relay_runs() {
    let Some((addr, pool, bus, server)) = spawn_sqlite_outbox().await else {
        return;
    };

    let mut sub = bus.subscribe(EventFilter::all()).await.unwrap();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();

    let resp = client
        .post(format!("http://{addr}/workspaces"))
        .json(&json!({"name": "sqlite-outbox-http"}))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success());
    assert!(outbox::count_pending(&pool).await.unwrap() >= 1);

    let no_event = tokio::time::timeout(Duration::from_millis(400), sub.next()).await;
    assert!(no_event.is_err(), "bus should not publish before relay");

    let relay = OutboxRelay::new(OutboxBackend::Sqlite(pool.clone()), bus.clone());
    relay.run_once().await.unwrap();

    let received = tokio::time::timeout(Duration::from_secs(5), sub.next())
        .await
        .expect("timeout")
        .expect("stream ended");
    let BusItem::Event(envelope) = received else {
        panic!("expected event");
    };
    assert!(envelope.log_id > 0);
    assert_eq!(outbox::count_pending(&pool).await.unwrap(), 0);

    server.abort();
}

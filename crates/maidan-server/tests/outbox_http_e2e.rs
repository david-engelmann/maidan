//! HTTP + Postgres outbox: publish defers bus delivery until relay runs.

mod common;

use std::{net::SocketAddr, sync::Arc, time::Duration};

use futures::StreamExt;
use maidan_artifacts::LocalFsStore;
use maidan_bus::{test_support::FailingBus, BusItem, EventBus, PostgresBus};
use maidan_server::{outbox_relay::OutboxRelay, router, AppState};
use maidan_store::{postgres::outbox, prelude::*, OutboxBackend};
use maidan_types::EventFilter;
use serde_json::json;
use sqlx::PgPool;
use testcontainers_modules::postgres::Postgres;

struct Harness {
    addr: SocketAddr,
    server: tokio::task::JoinHandle<()>,
    pool: PgPool,
    bus: Arc<PostgresBus>,
    _container: testcontainers::ContainerAsync<Postgres>,
}

async fn spawn_postgres_outbox() -> Option<Harness> {
    let (container, pool) = common::postgres_pool().await?;
    let bus = Arc::new(PostgresBus::connect(pool.clone()).await.ok()?);
    tokio::time::sleep(Duration::from_millis(200)).await;

    let store: Arc<dyn Store> = Arc::new(PostgresStore::new(pool.clone()));
    let search: Arc<dyn maidan_search::Search> =
        Arc::new(maidan_search::PostgresSearch::new(pool.clone()));
    let artifacts = Arc::new(LocalFsStore::new(tempfile::tempdir().ok()?.path()));

    let mut state = AppState::for_tests(store, artifacts, bus.clone(), search);
    state.outbox_relay = true;
    state.outbox_backend = Some(OutboxBackend::Postgres(pool.clone()));

    maidan_server::metrics::init();
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.ok()?;
    let addr = listener.local_addr().ok()?;
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    Some(Harness {
        addr,
        server,
        pool,
        bus,
        _container: container,
    })
}

#[tokio::test]
async fn http_mutation_defers_bus_until_outbox_relay_runs() {
    let Some(h) = spawn_postgres_outbox().await else {
        return;
    };

    let mut sub = h.bus.subscribe(EventFilter::all()).await.unwrap();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();

    let resp = client
        .post(format!("http://{}/workspaces", h.addr))
        .json(&json!({"name": "outbox-http-ws"}))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success());
    assert!(outbox::count_pending(&h.pool).await.unwrap() >= 1);

    let no_event = tokio::time::timeout(Duration::from_millis(400), sub.next()).await;
    assert!(no_event.is_err(), "bus should not publish before relay");

    let relay = OutboxRelay::new(OutboxBackend::Postgres(h.pool.clone()), h.bus.clone());
    relay.run_once().await.unwrap();

    let received = tokio::time::timeout(Duration::from_secs(5), sub.next())
        .await
        .expect("timeout")
        .expect("stream ended");
    let BusItem::Event(envelope) = received else {
        panic!("expected event");
    };
    assert!(envelope.log_id > 0);
    assert_eq!(outbox::count_pending(&h.pool).await.unwrap(), 0);

    h.server.abort();
}

#[tokio::test]
async fn metrics_scrape_reports_outbox_pending_on_postgres() {
    let Some(h) = spawn_postgres_outbox().await else {
        return;
    };

    let store = PostgresStore::new(h.pool.clone());
    let event = maidan_types::Event::WorkspaceCreated {
        occurred_at: chrono::Utc::now(),
        workspace: maidan_types::Workspace {
            id: maidan_types::WorkspaceId(uuid::Uuid::new_v4()),
            name: "metrics-pending".into(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            tombstoned_at: None,
        },
    };
    store.append_event(&event).await.unwrap();

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let body = client
        .get(format!("http://{}/metrics", h.addr))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();

    assert!(
        body.contains("maidan_outbox_pending"),
        "expected pending gauge in metrics body"
    );

    let relay = OutboxRelay::new(OutboxBackend::Postgres(h.pool.clone()), h.bus.clone());
    relay.run_once().await.unwrap();

    let body = client
        .get(format!("http://{}/metrics", h.addr))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(
        body.contains("maidan_outbox_relay_total"),
        "relay counter should appear after a successful relay tick"
    );
    assert!(body.contains("maidan_outbox_quarantined"));
    assert!(body.contains("maidan_outbox_oldest_pending_seconds"));

    h.server.abort();
}

#[tokio::test]
async fn relay_failure_keeps_pending_and_metrics_can_still_scrape() {
    let (container, pool) = match common::postgres_pool().await {
        Some(p) => p,
        None => return,
    };
    let bus = Arc::new(FailingBus::new("metrics-fail"));
    let store: Arc<dyn Store> = Arc::new(PostgresStore::new(pool.clone()));
    let search: Arc<dyn maidan_search::Search> =
        Arc::new(maidan_search::PostgresSearch::new(pool.clone()));
    let artifacts = Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path()));

    let mut state = AppState::for_tests(store.clone(), artifacts, bus.clone(), search);
    state.outbox_relay = true;
    state.outbox_backend = Some(OutboxBackend::Postgres(pool.clone()));

    maidan_server::metrics::init();
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    store
        .append_event(&maidan_types::Event::WorkspaceCreated {
            occurred_at: chrono::Utc::now(),
            workspace: maidan_types::Workspace {
                id: maidan_types::WorkspaceId(uuid::Uuid::new_v4()),
                name: "relay-fail".into(),
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
                tombstoned_at: None,
            },
        })
        .await
        .unwrap();

    let relay = OutboxRelay::with_max_attempts(OutboxBackend::Postgres(pool.clone()), bus, 2);
    relay.run_once().await.unwrap();
    relay.run_once().await.unwrap();
    assert_eq!(outbox::count_pending(&pool).await.unwrap(), 0);
    assert_eq!(outbox::count_quarantined(&pool).await.unwrap(), 1);

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let body = client
        .get(format!("http://{addr}/metrics"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(body.contains("maidan_outbox_pending"));
    assert!(body.contains("maidan_outbox_quarantined"));

    server.abort();
    drop(container);
}

#[tokio::test]
async fn replay_quarantined_outbox_row_via_http_then_relay_publishes() {
    let (container, pool) = match common::postgres_pool().await {
        Some(p) => p,
        None => return,
    };
    let bus = Arc::new(FailingBus::new("replay-once"));
    let store: Arc<dyn Store> = Arc::new(PostgresStore::new(pool.clone()));
    let search: Arc<dyn maidan_search::Search> =
        Arc::new(maidan_search::PostgresSearch::new(pool.clone()));
    let artifacts = Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path()));

    let mut state = AppState::for_tests(store.clone(), artifacts, bus.clone(), search);
    state.outbox_relay = true;
    state.outbox_backend = Some(OutboxBackend::Postgres(pool.clone()));

    maidan_server::metrics::init();
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let ws_id = maidan_types::WorkspaceId(uuid::Uuid::new_v4());
    store
        .append_event(&maidan_types::Event::WorkspaceCreated {
            occurred_at: chrono::Utc::now(),
            workspace: maidan_types::Workspace {
                id: ws_id,
                name: "replay-outbox".into(),
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
                tombstoned_at: None,
            },
        })
        .await
        .unwrap();

    let relay = OutboxRelay::with_max_attempts(OutboxBackend::Postgres(pool.clone()), bus, 2);
    relay.run_once().await.unwrap();
    relay.run_once().await.unwrap();
    assert_eq!(outbox::count_quarantined(&pool).await.unwrap(), 1);

    let row = sqlx::query_as::<_, (i64,)>("SELECT id FROM maidan_outbox LIMIT 1")
        .fetch_one(&pool)
        .await
        .unwrap();

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let resp = client
        .post(format!(
            "http://{addr}/workspaces/{}/outbox/{}/replay",
            ws_id.0, row.0
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::NO_CONTENT);
    assert_eq!(outbox::count_quarantined(&pool).await.unwrap(), 0);
    assert_eq!(outbox::count_pending(&pool).await.unwrap(), 1);

    let ok_bus = Arc::new(PostgresBus::connect(pool.clone()).await.unwrap());
    let relay2 = OutboxRelay::new(OutboxBackend::Postgres(pool.clone()), ok_bus);
    relay2.run_once().await.unwrap();
    assert_eq!(outbox::count_pending(&pool).await.unwrap(), 0);

    server.abort();
    drop(container);
}

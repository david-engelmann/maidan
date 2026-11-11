//! `GET /metrics` exposes Prometheus text (Track T.4).

use std::{net::SocketAddr, sync::Arc, time::Duration};

use maidan_artifacts::LocalFsStore;
use maidan_bus::{HydrateResult, HydrateStats};
use maidan_server::{router, AppState};
use maidan_store::{prelude::*, run_sqlite_migrations};

async fn spawn() -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let pool = maidan_search::sqlite_pool_options()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("foreign_keys");
    run_sqlite_migrations(&pool).await.expect("migrate");

    let store = Arc::new(SqliteStore::for_tests(pool.clone()));
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

    assert!(body.contains("maidan_http_request_total"));
    assert!(body.contains("maidan_indexer_last_event_age_seconds"));

    server.abort();
}

#[tokio::test]
async fn metrics_scrape_exports_hydrate_counters_when_stats_configured() {
    let pool = maidan_search::sqlite_pool_options()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("foreign_keys");
    run_sqlite_migrations(&pool).await.expect("migrate");

    let store = Arc::new(SqliteStore::for_tests(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let artifacts = Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path()));
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    let hydrate_stats = Arc::new(HydrateStats::default());
    hydrate_stats.record(HydrateResult::NotFound);

    maidan_server::metrics::init();
    let mut state = AppState::for_tests(store, artifacts, bus, search);
    state.bus_hydrate_stats = Some(hydrate_stats);

    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("client");

    let body = client
        .get(format!("http://{addr}/metrics"))
        .send()
        .await
        .expect("metrics")
        .text()
        .await
        .expect("body");

    assert!(body.contains("maidan_bus_notify_hydrate_total"));
    assert!(body.contains("not_found"));

    server.abort();
}

async fn spawn_state(state: AppState) -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let server = tokio::spawn(async move { axum::serve(listener, router(state)).await.unwrap() });
    (addr, server)
}

async fn metrics_text(client: &reqwest::Client, addr: SocketAddr) -> String {
    client
        .get(format!("http://{addr}/metrics"))
        .send()
        .await
        .expect("metrics")
        .text()
        .await
        .expect("body")
}

fn http_request_total_series(body: &str) -> Vec<String> {
    body.lines()
        .filter(|line| line.starts_with("maidan_http_request_total{"))
        .map(str::to_string)
        .collect()
}

fn method_labels(body: &str) -> std::collections::BTreeSet<String> {
    let mut labels = std::collections::BTreeSet::new();
    for line in http_request_total_series(body) {
        if let Some(start) = line.find("method=\"") {
            let rest = &line[start + 8..];
            if let Some(end) = rest.find('"') {
                labels.insert(rest[..end].to_string());
            }
        }
    }
    labels
}

const CLOSED_METHODS: &[&str] = &[
    "GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS", "other",
];

#[tokio::test]
async fn two_hundred_random_methods_add_no_new_series() {
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
    let before = metrics_text(&client, addr).await;
    let series_before = http_request_total_series(&before).len();

    for i in 0..200 {
        let method = reqwest::Method::from_bytes(format!("M{i:03}").as_bytes()).unwrap();
        let _ = client
            .request(method, format!("http://{addr}/health/live"))
            .send()
            .await
            .expect("custom method");
    }

    let after = metrics_text(&client, addr).await;
    let methods = method_labels(&after);
    for method in &methods {
        assert!(
            CLOSED_METHODS.contains(&method.as_str()),
            "open method label {method}"
        );
    }
    assert!(methods.contains("other"), "{methods:?}");
    let series_after = http_request_total_series(&after).len();
    assert!(
        series_after <= series_before + 1,
        "200 random methods added {} series (before {series_before}, after {series_after})",
        series_after.saturating_sub(series_before)
    );

    server.abort();
}

#[tokio::test]
async fn http_metrics_use_the_matched_route_template() {
    let (addr, server) = spawn().await;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("client");

    let _ = client
        .get(format!("http://{addr}/threads/tid-example/votes"))
        .send()
        .await
        .expect("thread votes");

    let body = metrics_text(&client, addr).await;
    assert!(
        body.contains("route=\"/threads/{tid}/votes\""),
        "expected matched-path template in {body}"
    );
    assert!(
        !body.contains("route=\"/threads/tid-example/votes\""),
        "raw path leaked into a route label: {body}"
    );

    server.abort();
}

#[tokio::test]
async fn metrics_scrape_requires_the_configured_bearer_token() {
    let pool = maidan_search::sqlite_pool_options()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("foreign_keys");
    run_sqlite_migrations(&pool).await.expect("migrate");

    let store = Arc::new(SqliteStore::for_tests(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let artifacts = Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path()));
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    maidan_server::metrics::init();
    let mut state = AppState::for_tests(store, artifacts, bus, search);
    state.metrics_token = Some(Arc::from("s3cret"));

    let (addr, server) = spawn_state(state).await;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("client");

    let unauth = client
        .get(format!("http://{addr}/metrics"))
        .send()
        .await
        .expect("unauth");
    assert_eq!(unauth.status(), reqwest::StatusCode::UNAUTHORIZED);

    let wrong = client
        .get(format!("http://{addr}/metrics"))
        .header("Authorization", "Bearer wrong")
        .send()
        .await
        .expect("wrong");
    assert_eq!(wrong.status(), reqwest::StatusCode::UNAUTHORIZED);

    let ok = client
        .get(format!("http://{addr}/metrics"))
        .header("Authorization", "Bearer s3cret")
        .send()
        .await
        .expect("ok");
    assert_eq!(ok.status(), reqwest::StatusCode::OK);
    let body = ok.text().await.expect("body");
    assert!(
        body.contains("maidan_http_request_total")
            || body.contains("maidan_indexer_last_event_age_seconds")
    );

    server.abort();
}

#[tokio::test]
async fn metrics_are_not_on_the_public_router_when_closed() {
    let pool = maidan_search::sqlite_pool_options()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("foreign_keys");
    run_sqlite_migrations(&pool).await.expect("migrate");

    let store = Arc::new(SqliteStore::for_tests(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let artifacts = Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path()));
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    maidan_server::metrics::init();
    let mut state = AppState::for_tests(store, artifacts, bus, search);
    state.metrics_on_public_router = false;

    let (addr, server) = spawn_state(state).await;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("client");

    let response = client
        .get(format!("http://{addr}/metrics"))
        .send()
        .await
        .expect("metrics");
    assert_eq!(response.status(), reqwest::StatusCode::NOT_FOUND);

    server.abort();
}

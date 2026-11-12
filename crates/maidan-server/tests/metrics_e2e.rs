//! `GET /metrics` exposes Prometheus text (Track T.4).

use std::{collections::BTreeSet, net::SocketAddr, sync::Arc, time::Duration};

use maidan_artifacts::LocalFsStore;
use maidan_bus::{HydrateResult, HydrateStats};
use maidan_server::{metrics::MetricsAccess, router, AppState};
use maidan_store::{prelude::*, run_sqlite_migrations};

async fn spawn() -> (SocketAddr, tokio::task::JoinHandle<()>) {
    spawn_with(MetricsAccess::Open).await
}

async fn spawn_with(access: MetricsAccess) -> (SocketAddr, tokio::task::JoinHandle<()>) {
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
    state.metrics_access = access;
    let app = router(state);

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

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("client")
}

async fn scrape(client: &reqwest::Client, addr: SocketAddr) -> String {
    let response = client
        .get(format!("http://{addr}/metrics"))
        .send()
        .await
        .expect("metrics");
    assert_eq!(response.status(), 200);
    response.text().await.expect("body")
}

/// The series (name and labels, without the value) of the HTTP metrics that
/// hold every one of `labels`.
fn http_series(body: &str, labels: &[&str]) -> BTreeSet<String> {
    body.lines()
        .filter(|line| line.starts_with("http_server_request"))
        .filter(|line| labels.iter().all(|label| line.contains(label)))
        .filter_map(|line| line.rsplit_once(' ').map(|(series, _)| series.to_string()))
        .collect()
}

/// A random extension method, a valid HTTP token no client sends twice.
fn invented_method() -> reqwest::Method {
    let name = format!("X{}", uuid::Uuid::now_v7().simple()).to_uppercase();
    reqwest::Method::from_bytes(name.as_bytes()).expect("a valid method token")
}

/// HTTP lets a client send any token as its method, and the recorder keeps
/// every series it has seen. Before the method label was a fixed set, 200
/// requests with random methods added 2,000 series that were never released.
#[tokio::test]
async fn two_hundred_random_methods_add_no_series() {
    let (addr, server) = spawn().await;
    let client = client();
    let ours = ["method=\"other\"", "route=\"/health/live\""];

    // The first invented method creates the `other` series for this route.
    let warmup = client
        .request(invented_method(), format!("http://{addr}/health/live"))
        .send()
        .await
        .expect("warmup");
    assert_eq!(warmup.status(), 405);
    let before = http_series(&scrape(&client, addr).await, &ours);
    assert!(!before.is_empty(), "the warm-up request was not recorded");

    let mut sent = Vec::new();
    for _ in 0..200 {
        let method = invented_method();
        sent.push(method.to_string());
        client
            .request(method, format!("http://{addr}/health/live"))
            .send()
            .await
            .expect("request");
    }
    let body = scrape(&client, addr).await;
    assert_eq!(
        http_series(&body, &ours),
        before,
        "random methods added series"
    );
    for method in &sent {
        assert!(
            !body.contains(method.as_str()),
            "{method} reached the exposition"
        );
    }
    // Every method label anywhere in the exposition is one of the fixed set,
    // whatever other tests in this binary sent.
    let fixed = [
        "GET", "POST", "PUT", "PATCH", "DELETE", "HEAD", "OPTIONS", "other",
    ];
    for series in http_series(&body, &[]) {
        let Some((_, rest)) = series.split_once("method=\"") else {
            continue;
        };
        let label = rest.split('"').next().unwrap_or_default();
        assert!(fixed.contains(&label), "{series}");
    }

    server.abort();
}

/// The route label is the template the router matched, so ids in the path
/// never become series; a path no route matches is `unmatched`.
#[tokio::test]
async fn the_route_label_is_the_matched_template_never_the_path() {
    let (addr, server) = spawn().await;
    let client = client();
    let id = uuid::Uuid::now_v7();
    client
        .get(format!("http://{addr}/threads/{id}/result"))
        .send()
        .await
        .expect("thread result");
    let unknown = uuid::Uuid::now_v7();
    client
        .get(format!("http://{addr}/no-such-route-{unknown}"))
        .send()
        .await
        .expect("unknown path");

    let body = scrape(&client, addr).await;
    assert!(
        !http_series(&body, &["route=\"/threads/{id}/result\""]).is_empty(),
        "no series for the matched template"
    );
    assert!(
        !http_series(&body, &["route=\"unmatched\""]).is_empty(),
        "no series for the unmatched path"
    );
    for raw in [id.to_string(), unknown.to_string()] {
        assert!(!body.contains(&raw), "the raw path reached a label: {raw}");
    }

    server.abort();
}

const SCRAPE_TOKEN: &str = "a-scrape-token-for-this-test-0123456789";

/// With `MAIDAN_METRICS_TOKEN` set, the scrape answers only its bearer.
#[tokio::test]
async fn a_token_puts_the_scrape_behind_its_bearer() {
    let access = MetricsAccess::from_settings(true, Some(SCRAPE_TOKEN.into())).expect("access");
    let (addr, server) = spawn_with(access).await;
    let client = client();
    let url = format!("http://{addr}/metrics");

    let anonymous = client.get(&url).send().await.expect("anonymous");
    assert_eq!(anonymous.status(), 401);
    let wrong = client
        .get(&url)
        .bearer_auth("not-the-scrape-token-but-just-as-long-0123")
        .send()
        .await
        .expect("wrong token");
    assert_eq!(wrong.status(), 401);
    let scraper = client
        .get(&url)
        .bearer_auth(SCRAPE_TOKEN)
        .send()
        .await
        .expect("scraper");
    assert_eq!(scraper.status(), 200);
    assert!(scraper
        .text()
        .await
        .expect("body")
        .contains("maidan_indexer_last_event_age_seconds"));

    server.abort();
}

/// Production with no token configured serves no scrape at all.
#[tokio::test]
async fn production_without_a_token_serves_no_scrape() {
    let access = MetricsAccess::from_settings(true, None).expect("access");
    let (addr, server) = spawn_with(access).await;
    let response = client()
        .get(format!("http://{addr}/metrics"))
        .bearer_auth(SCRAPE_TOKEN)
        .send()
        .await
        .expect("scrape");
    assert_eq!(response.status(), 404);

    server.abort();
}

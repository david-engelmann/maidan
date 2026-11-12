//! Dynamic client registration is charged per address
//! (`MAIDAN_OAUTH_REGISTER_RATE_LIMIT_*`, `docs/OAuth.md`): every
//! registration is a row, so one address cannot flood the table, whatever
//! names and redirects it asserts, and another address keeps its own budget.

use std::sync::{Arc, Once};

use maidan_artifacts::LocalFsStore;
use maidan_server::oauth::register::DynamicRegistration;
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use reqwest::StatusCode;
use serde_json::json;

static ENV: Once = Once::new();

/// The limiter's buckets are process-global; each test keys its traffic by
/// its own forwarded addresses.
fn env() {
    #[allow(unsafe_code)] // edition 2024 makes env writes unsafe; test setup only
    ENV.call_once(|| unsafe {
        std::env::set_var("MAIDAN_OAUTH_REGISTER_RATE_LIMIT_MAX", "3");
        std::env::set_var("MAIDAN_OAUTH_REGISTER_RATE_LIMIT_WINDOW_SECS", "3600");
        // The forwarded address names the client in these tests.
        std::env::set_var("MAIDAN_TRUSTED_PROXY_HOPS", "1");
    });
}

async fn spawn() -> String {
    env();
    let pool = maidan_search::sqlite_pool_options()
        .max_connections(4)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
    let search: Arc<dyn maidan_search::Search> =
        Arc::new(maidan_search::SqliteSearch::new(pool.clone()));
    let dir = tempfile::tempdir().unwrap();
    let mut state = AppState::new(
        store,
        Arc::new(LocalFsStore::new(dir.path())),
        Arc::new(maidan_bus::InMemoryBus::new()),
        search,
        Arc::new(maidan_search::HashV1Provider),
        false,
        false,
        FederationRuntime::new(true, None),
        Arc::new(std::sync::atomic::AtomicI64::new(0)),
        None,
    );
    state.public_origin = Some("https://maidan.example.com".into());
    state.oauth_dynamic_registration = Some(DynamicRegistration::default());
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .unwrap()
    });
    format!("http://{addr}")
}

async fn register(base: &str, xff: &str, n: usize) -> reqwest::Response {
    reqwest::Client::new()
        .post(format!("{base}/oauth/register"))
        .header("x-forwarded-for", xff)
        .json(&json!({
            "client_name": format!("Flood {n}"),
            "redirect_uris": [format!("http://127.0.0.1/cb{n}")],
        }))
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn one_address_meets_the_registration_limit_and_another_keeps_its_own() {
    let base = spawn().await;
    for n in 0..3 {
        assert_eq!(
            register(&base, "10.7.0.1", n).await.status(),
            StatusCode::CREATED,
            "registration {n}"
        );
    }
    let fourth = register(&base, "10.7.0.1", 3).await;
    assert_eq!(fourth.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(fourth.headers().contains_key(reqwest::header::RETRY_AFTER));
    assert_eq!(
        register(&base, "10.7.0.2", 4).await.status(),
        StatusCode::CREATED,
        "another address has its own budget"
    );
}

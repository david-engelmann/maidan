//! The OAuth authorization-server surface carries its own rate limits,
//! per client id and per address (`docs/OAuth.md`): the token, revoke,
//! authorize and consent endpoints are the server's credential-guessing
//! surface, so they are charged under tighter buckets than the API at
//! large — and under buckets a busy API client cannot spend.

use std::sync::{Arc, Once};

use maidan_artifacts::LocalFsStore;
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use reqwest::StatusCode;
use sqlx::sqlite::SqlitePoolOptions;

const ORIGIN: &str = "https://maidan.example.com";

static ENV: Once = Once::new();

/// One set of limits for the whole file: the limiter's buckets are
/// process-global, so each test keys its traffic distinctly (its own
/// client ids and forwarded addresses) and shares these values.
fn env() {
    ENV.call_once(|| unsafe {
        std::env::set_var("MAIDAN_OAUTH_CLIENT_RATE_LIMIT_MAX", "3");
        std::env::set_var("MAIDAN_OAUTH_CLIENT_RATE_LIMIT_WINDOW_SECS", "60");
        std::env::set_var("MAIDAN_OAUTH_IP_RATE_LIMIT_MAX", "5");
        std::env::set_var("MAIDAN_OAUTH_IP_RATE_LIMIT_WINDOW_SECS", "60");
        // The forwarded address names the client in these tests.
        std::env::set_var("MAIDAN_TRUSTED_PROXY_HOPS", "1");
    });
}

async fn spawn() -> String {
    env();
    let pool = SqlitePoolOptions::new()
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
    state.public_origin = Some(ORIGIN.into());
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    // As production serves it: the peer address is what an address-keyed
    // bucket is charged under when no proxy hops are declared.
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

/// One token-endpoint attempt with a useless code, as `client_id`
/// authenticating with Basic from forwarded address `xff`. The handler's
/// refusal (401, an unregistered client) is beside the point; only the
/// limiter's answer matters.
async fn token_attempt(base: &str, xff: &str, client_id: &str) -> StatusCode {
    reqwest::Client::new()
        .post(format!("{base}/oauth/token"))
        .header("x-forwarded-for", xff)
        .basic_auth(client_id, Some("wrong-secret"))
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", "not-a-code"),
            ("redirect_uri", "https://client.example/callback"),
        ])
        .send()
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn a_client_meets_its_own_oauth_limit() {
    let base = spawn().await;
    for _ in 0..3 {
        assert_eq!(
            token_attempt(&base, "10.9.1.1", "rl-client-a").await,
            StatusCode::UNAUTHORIZED
        );
    }
    let resp = reqwest::Client::new()
        .post(format!("{base}/oauth/token"))
        .header("x-forwarded-for", "10.9.1.1")
        .basic_auth("rl-client-a", Some("wrong-secret"))
        .form(&[("grant_type", "authorization_code"), ("code", "x")])
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(resp.headers().contains_key(reqwest::header::RETRY_AFTER));
    // Another client from the same address still has its own bucket.
    assert_eq!(
        token_attempt(&base, "10.9.1.1", "rl-client-b").await,
        StatusCode::UNAUTHORIZED
    );
}

#[tokio::test]
async fn an_address_meets_the_oauth_ip_limit_across_clients() {
    let base = spawn().await;
    let client = reqwest::Client::new();
    for n in 0..5 {
        let status = client
            .post(format!("{base}/oauth/token"))
            .header("x-forwarded-for", "10.9.0.1")
            .form(&[
                ("grant_type", "authorization_code"),
                ("code", "not-a-code"),
                ("client_id", &format!("rl-fleet-{n}")),
            ])
            .send()
            .await
            .unwrap()
            .status();
        assert_ne!(status, StatusCode::TOO_MANY_REQUESTS, "attempt {n}");
    }
    let sixth = client
        .post(format!("{base}/oauth/token"))
        .header("x-forwarded-for", "10.9.0.1")
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", "not-a-code"),
            ("client_id", "rl-fleet-new"),
        ])
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(sixth, StatusCode::TOO_MANY_REQUESTS);
    // The same new client from another address is not limited.
    let other = client
        .post(format!("{base}/oauth/token"))
        .header("x-forwarded-for", "10.9.0.2")
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", "not-a-code"),
            ("client_id", "rl-fleet-new"),
        ])
        .send()
        .await
        .unwrap()
        .status();
    assert_ne!(other, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn a_body_client_id_names_the_client_bucket() {
    let base = spawn().await;
    let client = reqwest::Client::new();
    let revoke = |xff: &str| {
        client
            .post(format!("{base}/oauth/revoke"))
            .header("x-forwarded-for", xff)
            .form(&[("token", "not-a-token"), ("client_id", "rl-body-client")])
    };
    for _ in 0..3 {
        assert_ne!(
            revoke("10.9.3.1").send().await.unwrap().status(),
            StatusCode::TOO_MANY_REQUESTS
        );
    }
    assert_eq!(
        revoke("10.9.3.1").send().await.unwrap().status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    // The client bucket is not per-address: a fresh address changes nothing.
    assert_eq!(
        revoke("10.9.3.2").send().await.unwrap().status(),
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
async fn the_authorize_endpoint_is_limited_per_client() {
    let base = spawn().await;
    let client = reqwest::Client::new();
    let url = format!(
        "{base}/oauth/authorize?response_type=code&client_id=rl-web-client&redirect_uri=https%3A%2F%2Fclient.example%2Fcallback"
    );
    for _ in 0..3 {
        let status = client
            .get(&url)
            .header("x-forwarded-for", "10.9.2.1")
            .send()
            .await
            .unwrap()
            .status();
        assert_ne!(status, StatusCode::TOO_MANY_REQUESTS);
    }
    let status = client
        .get(&url)
        .header("x-forwarded-for", "10.9.2.1")
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn the_consent_page_is_limited_per_address_and_the_api_is_not_charged() {
    let base = spawn().await;
    let client = reqwest::Client::new();
    for _ in 0..5 {
        let status = client
            .get(format!("{base}/ui/oauth/consent"))
            .header("x-forwarded-for", "10.9.4.1")
            .send()
            .await
            .unwrap()
            .status();
        assert_ne!(status, StatusCode::TOO_MANY_REQUESTS);
    }
    let status = client
        .get(format!("{base}/ui/oauth/consent"))
        .header("x-forwarded-for", "10.9.4.1")
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    // The OAuth buckets belong to the OAuth surface: the API at large from
    // the same spent address is answered on its own terms (here, a 401).
    let status = client
        .get(format!("{base}/workspaces"))
        .header("x-forwarded-for", "10.9.4.1")
        .send()
        .await
        .unwrap()
        .status();
    assert_ne!(status, StatusCode::TOO_MANY_REQUESTS);
}

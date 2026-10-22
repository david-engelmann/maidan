//! Next 26 rate limits: the waitlist and sign-up surface is charged per
//! address, and a waitlist join is also charged per email domain, so one
//! network cannot flood the list and one domain cannot be stuffed with
//! addresses. The limits here are tightened through the environment the
//! specs resolve, in this binary only.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use maidan_artifacts::LocalFsStore;
use maidan_server::{
    oidc::{OidcRuntime, OidcSettings},
    router, AppState, FederationRuntime,
};
use maidan_store::{prelude::*, run_sqlite_migrations};
use reqwest::StatusCode;
use serde_json::json;

async fn spawn() -> String {
    // SAFETY: test setup. Maidan reads this variable only through std,
    // whose environment lock serializes the read with this write.
    unsafe { std::env::set_var("MAIDAN_SIGNUP_RATE_LIMIT_MAX", "3") };
    // SAFETY: test setup. Maidan reads this variable only through std,
    // whose environment lock serializes the read with this write.
    unsafe { std::env::set_var("MAIDAN_SIGNUP_RATE_LIMIT_WINDOW_SECS", "3600") };
    // SAFETY: test setup. Maidan reads this variable only through std,
    // whose environment lock serializes the read with this write.
    unsafe { std::env::set_var("MAIDAN_WAITLIST_DOMAIN_RATE_LIMIT_MAX", "2") };
    // SAFETY: test setup. Maidan reads this variable only through std,
    // whose environment lock serializes the read with this write.
    unsafe { std::env::set_var("MAIDAN_WAITLIST_DOMAIN_RATE_LIMIT_WINDOW_SECS", "3600") };
    // The limiter's buckets are process-global, so the two tests steer
    // themselves into separate address buckets with forwarded addresses.
    // SAFETY: test setup. Maidan reads this variable only through std,
    // whose environment lock serializes the read with this write.
    unsafe { std::env::set_var("MAIDAN_TRUSTED_PROXY_HOPS", "1") };
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
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
        Arc::new(AtomicI64::new(0)),
        None,
    );
    // The waitlist surface answers 404 unless sign-up is on, so the limiter
    // tests run against a server with mock OIDC and sign-up switched on.
    state.oidc = Some(Arc::new(OidcRuntime {
        settings: OidcSettings {
            enabled: true,
            mock: true,
            issuer: "https://idp.example".into(),
            redirect_uri: "http://127.0.0.1/auth/oidc/callback".into(),
            auto_provision: false,
            link_email: false,
            session_ttl_secs: 3600,
            pending_ttl_secs: 600,
            cookie_secure: false,
            post_logout_redirect_uri: None,
            first_admin_mint: true,
            auto_mint: false,
            signup: true,
            signup_email_domains: Vec::new(),
        },
        session_secret: b"signup-rate-limit-e2e-secret-32b".as_slice().into(),
        client: None,
        http_client: None,
        end_session_url: None,
        logout_client_id: None,
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let app = router(state);
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap()
    });
    std::mem::forget(dir);
    format!("http://{addr}")
}

async fn join(client: &reqwest::Client, base: &str, ip: &str, email: &str) -> StatusCode {
    client
        .post(format!("{base}/waitlist"))
        .header("x-forwarded-for", ip)
        .json(&json!({"email": email, "use_case": "swarms", "consent": true}))
        .send()
        .await
        .unwrap()
        .status()
}

#[tokio::test]
async fn the_waitlist_refuses_a_fourth_join_from_one_address() {
    let base = spawn().await;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    // Three domains, one address: the address budget (3) is the binding one.
    assert_eq!(
        join(&client, &base, "10.0.0.1", "a@one.example").await,
        StatusCode::CREATED
    );
    assert_eq!(
        join(&client, &base, "10.0.0.1", "b@two.example").await,
        StatusCode::CREATED
    );
    assert_eq!(
        join(&client, &base, "10.0.0.1", "c@three.example").await,
        StatusCode::CREATED
    );
    assert_eq!(
        join(&client, &base, "10.0.0.1", "d@four.example").await,
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
async fn the_waitlist_refuses_a_third_join_for_one_email_domain() {
    let base = spawn().await;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    // The domain budget (2) binds before the address budget (3). The second
    // address is padded: the handler stores it trimmed, and the limiter
    // must charge the same bucket, not a fresh one for the padding.
    assert_eq!(
        join(&client, &base, "10.0.0.2", "a@same.example").await,
        StatusCode::CREATED
    );
    assert_eq!(
        join(&client, &base, "10.0.0.2", "b@same.example ").await,
        StatusCode::CREATED
    );
    assert_eq!(
        join(&client, &base, "10.0.0.2", "c@same.example").await,
        StatusCode::TOO_MANY_REQUESTS
    );
}

/// A raw chunked POST: no Content-Length at all. The domain charge must
/// still land, so two chunked joins exhaust the domain budget.
async fn join_chunked(base: &str, ip: &str, email: &str) -> StatusCode {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let addr = base.trim_start_matches("http://");
    let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
    let body = json!({"email": email, "use_case": "swarms", "consent": true}).to_string();
    let request = format!(
        "POST /waitlist HTTP/1.1\r\nhost: {addr}\r\ncontent-type: application/json\r\nx-forwarded-for: {ip}\r\ntransfer-encoding: chunked\r\nconnection: close\r\n\r\n{:x}\r\n{body}\r\n0\r\n\r\n",
        body.len()
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).await.unwrap();
    let code: u16 = response
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .expect("status line");
    StatusCode::from_u16(code).unwrap()
}

#[tokio::test]
async fn a_chunked_join_is_charged_to_its_domain_too() {
    let base = spawn().await;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    assert_eq!(
        join_chunked(&base, "10.0.0.3", "a@chunked.example").await,
        StatusCode::CREATED
    );
    assert_eq!(
        join_chunked(&base, "10.0.0.3", "b@chunked.example").await,
        StatusCode::CREATED
    );
    assert_eq!(
        join(&client, &base, "10.0.0.3", "c@chunked.example").await,
        StatusCode::TOO_MANY_REQUESTS
    );
}

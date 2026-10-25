//! Turnstile on the sign-up surface: with a secret configured, a waitlist
//! join and a sign-up start need a token that siteverify accepts for this
//! hostname and step; without one, nothing changes. Siteverify is a mock on
//! loopback here, reached through the egress guard's development escape
//! hatch.
//!
//! The waitlist and the sign-up start are signed-out, instance-wide routes:
//! no workspace is in scope until a sign-up creates one, so there is no
//! second tenant to keep out.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc, Mutex},
};

use axum::{routing::post, Form, Json, Router};
use maidan_artifacts::LocalFsStore;
use maidan_server::{
    oidc::{OidcRuntime, OidcSettings},
    router,
    turnstile::{TurnstileSecret, TurnstileSettings},
    AppState, FederationRuntime,
};
use maidan_store::{prelude::*, run_sqlite_migrations};
use reqwest::{redirect::Policy, StatusCode};
use serde_json::{json, Value};

const SECRET: &str = "turnstile-e2e-secret";
const HOST: &str = "maidan.example";

/// What the mock siteverify saw: every (secret, token) pair posted to it.
type Seen = Arc<Mutex<Vec<(String, String)>>>;

// The mock siteverify is a stand-in for Cloudflare, not a maidan route.
#[allow(clippy::disallowed_methods)]
async fn spawn_siteverify() -> (String, Seen) {
    let seen: Seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    let app = Router::new().route(
        "/siteverify",
        post(move |Form(form): Form<std::collections::HashMap<String, String>>| {
            let log = log.clone();
            async move {
                let secret = form.get("secret").cloned().unwrap_or_default();
                let token = form.get("response").cloned().unwrap_or_default();
                log.lock().unwrap().push((secret.clone(), token.clone()));
                let (success, hostname, action) = match (secret == SECRET, token.as_str()) {
                    (false, _) => (false, HOST, ""),
                    (true, "good-waitlist") => (true, HOST, "waitlist"),
                    (true, "good-signup") => (true, HOST, "signup"),
                    (true, "other-host") => (true, "attacker.example", "waitlist"),
                    _ => (false, HOST, ""),
                };
                Json(json!({
                    "success": success,
                    "hostname": hostname,
                    "action": action,
                    "error-codes": if success { json!([]) } else { json!(["timeout-or-duplicate"]) },
                }))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}/siteverify"), seen)
}

async fn spawn(turnstile: Option<TurnstileSettings>) -> String {
    // SAFETY: test setup. Maidan reads these variables only through std,
    // whose environment lock serializes the read with this write.
    unsafe {
        std::env::set_var("MAIDAN_ALLOW_PRIVATE_EGRESS", "1");
        std::env::set_var("MAIDAN_SIGNUP_RATE_LIMIT_MAX", "1000");
        std::env::set_var("MAIDAN_WAITLIST_DOMAIN_RATE_LIMIT_MAX", "1000");
    }
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
            signup_open: true,
            emails_verified: false,
        },
        session_secret: b"turnstile-e2e-session-secret-32b".as_slice().into(),
        client: None,
        http_client: None,
        end_session_url: None,
        logout_client_id: None,
    }));
    state.turnstile = turnstile.map(Arc::new);
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

fn settings(siteverify_url: String) -> TurnstileSettings {
    TurnstileSettings {
        site_key: "site-key-public".into(),
        secret: TurnstileSecret::new(SECRET),
        hostname: HOST.into(),
        siteverify_url,
    }
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .redirect(Policy::none())
        .build()
        .unwrap()
}

async fn join(base: &str, email: &str, token: Option<&str>) -> (StatusCode, Value) {
    let mut body = json!({ "email": email, "use_case": "a review swarm", "consent": true });
    if let Some(token) = token {
        body["turnstile_token"] = json!(token);
    }
    let res = client()
        .post(format!("{base}/waitlist"))
        .json(&body)
        .send()
        .await
        .unwrap();
    let status = res.status();
    (status, res.json().await.unwrap_or(Value::Null))
}

#[tokio::test]
async fn a_waitlist_join_without_a_token_is_refused_and_siteverify_is_not_called() {
    let (url, seen) = spawn_siteverify().await;
    let base = spawn(Some(settings(url))).await;
    let (status, body) = join(&base, "missing@one.example", None).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(
        body.to_string().contains("complete the human check"),
        "{body}"
    );
    let (status, _) = join(&base, "blank@one.example", Some("  ")).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_waitlist_join_with_a_token_siteverify_rejects_is_refused() {
    let (url, _) = spawn_siteverify().await;
    let base = spawn(Some(settings(url))).await;
    let (status, body) = join(&base, "bad@two.example", Some("spent-or-forged")).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert!(body.to_string().contains("not accepted"), "{body}");
}

#[tokio::test]
async fn a_waitlist_join_with_a_valid_token_is_accepted_and_sends_the_secret_to_siteverify() {
    let (url, seen) = spawn_siteverify().await;
    let base = spawn(Some(settings(url))).await;
    let (status, _) = join(&base, "good@three.example", Some("good-waitlist")).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(
        seen.lock().unwrap().as_slice(),
        &[(SECRET.to_string(), "good-waitlist".to_string())]
    );
}

#[tokio::test]
async fn a_token_solved_on_another_hostname_or_for_another_step_is_refused() {
    let (url, _) = spawn_siteverify().await;
    let base = spawn(Some(settings(url))).await;
    let (status, _) = join(&base, "host@four.example", Some("other-host")).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, _) = join(&base, "step@four.example", Some("good-signup")).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn an_unreachable_siteverify_refuses_the_join_rather_than_letting_it_through() {
    // Bind and drop a listener so the port is closed.
    let closed = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = closed.local_addr().unwrap();
    drop(closed);
    let base = spawn(Some(settings(format!("http://{addr}/siteverify")))).await;
    let (status, body) = join(&base, "down@five.example", Some("good-waitlist")).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
    assert!(!body.to_string().contains(SECRET), "{body}");
}

#[tokio::test]
async fn a_siteverify_target_the_egress_guard_refuses_fails_closed() {
    // The egress guard refuses a target with credentials in it, escape
    // hatch or not, and a join it refuses is refused too.
    let base = spawn(Some(settings(
        "http://user:pass@127.0.0.1:9/siteverify".to_string(),
    )))
    .await;
    let (status, _) = join(&base, "guard@six.example", Some("good-waitlist")).await;
    assert_eq!(status, StatusCode::BAD_GATEWAY);
}

#[tokio::test]
async fn a_sign_up_start_without_a_token_is_refused_for_an_api_caller() {
    let (url, _) = spawn_siteverify().await;
    let base = spawn(Some(settings(url))).await;
    let res = client()
        .get(format!("{base}/auth/oidc/signup"))
        .header("accept", "application/json")
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn a_browser_following_a_sign_up_link_without_a_token_is_sent_to_the_console_check() {
    let (url, seen) = spawn_siteverify().await;
    let base = spawn(Some(settings(url))).await;
    let res = client()
        .get(format!("{base}/auth/oidc/signup?return_to=/ui/"))
        .header("accept", "text/html,application/xhtml+xml")
        .send()
        .await
        .unwrap();
    assert!(res.status().is_redirection(), "{}", res.status());
    let location = res.headers()["location"].to_str().unwrap().to_string();
    assert_eq!(location, "/ui/?signup_check=1&return_to=%2Fui%2F");
    assert!(seen.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_sign_up_start_with_a_rejected_or_waitlist_token_is_refused() {
    let (url, _) = spawn_siteverify().await;
    let base = spawn(Some(settings(url))).await;
    for token in ["spent-or-forged", "good-waitlist"] {
        let res = client()
            .get(format!("{base}/auth/oidc/signup?turnstile={token}"))
            .header("accept", "text/html")
            .send()
            .await
            .unwrap();
        assert_eq!(res.status(), StatusCode::FORBIDDEN, "{token}");
    }
}

#[tokio::test]
async fn a_sign_up_start_with_a_valid_token_goes_on_to_the_provider() {
    let (url, _) = spawn_siteverify().await;
    let base = spawn(Some(settings(url))).await;
    let res = client()
        .get(format!("{base}/auth/oidc/signup?turnstile=good-signup"))
        .send()
        .await
        .unwrap();
    assert!(res.status().is_redirection(), "{}", res.status());
    let location = res.headers()["location"].to_str().unwrap();
    assert!(
        location.starts_with("/auth/oidc/callback?state="),
        "{location}"
    );
}

#[tokio::test]
async fn with_turnstile_on_discovery_names_the_site_key_and_the_csp_allows_only_the_challenge_origin(
) {
    let (url, _) = spawn_siteverify().await;
    let base = spawn(Some(settings(url))).await;
    let doc: Value = client()
        .get(format!("{base}/.well-known/maidan.json"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(doc["auth"]["turnstile_site_key"], "site-key-public");
    assert!(!doc.to_string().contains(SECRET));
    for path in ["/", "/ui/"] {
        let res = client().get(format!("{base}{path}")).send().await.unwrap();
        let csp = res.headers()["content-security-policy"].to_str().unwrap();
        assert!(
            csp.contains("script-src 'self' https://challenges.cloudflare.com;"),
            "{path}: {csp}"
        );
        assert!(
            csp.contains("frame-src https://challenges.cloudflare.com;"),
            "{path}: {csp}"
        );
    }
}

#[tokio::test]
async fn with_turnstile_unset_nothing_asks_for_a_token_and_the_csp_is_unchanged() {
    let base = spawn(None).await;
    let (status, _) = join(&base, "plain@seven.example", None).await;
    assert_eq!(status, StatusCode::CREATED);
    let res = client()
        .get(format!("{base}/auth/oidc/signup"))
        .header("accept", "text/html")
        .send()
        .await
        .unwrap();
    let location = res.headers()["location"].to_str().unwrap();
    assert!(
        location.starts_with("/auth/oidc/callback?state="),
        "{location}"
    );
    let doc: Value = client()
        .get(format!("{base}/.well-known/maidan.json"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(doc["auth"].get("turnstile_site_key").is_none());
    let res = client().get(format!("{base}/")).send().await.unwrap();
    assert_eq!(
        res.headers()["content-security-policy"].to_str().unwrap(),
        maidan_server::app::LANDING_CSP
    );
    let res = client().get(format!("{base}/ui/")).send().await.unwrap();
    assert_eq!(
        res.headers()["content-security-policy"].to_str().unwrap(),
        maidan_server::app::BOARD_UI_CSP
    );
}

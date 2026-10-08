//! Outbound GitHub calls authenticated as a GitHub App (Open Work Next 22).
//!
//! The real `GithubApiClient` runs against a loopback fake of GitHub. The fake
//! verifies the app JWT's RS256 signature with the app's public key, checks
//! its claims, and hands out numbered installation tokens, so every test can
//! tell which exchange a request's bearer came from. The boot tests run the
//! real binary, because the app config is read in `main`.
//!
//! The app key is generated when the tests run (`github_app_key`), so no
//! private key is committed and none is registered with GitHub.

// The fake GitHub is a plain axum server, not the API (see clippy.toml).
#![allow(clippy::disallowed_methods)]

use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::{
    extract::State,
    http::{HeaderMap, Method, StatusCode, Uri},
    response::IntoResponse,
    routing::any,
    Json, Router,
};
use base64::Engine as _;
use maidan_server::github::{GithubApiClient, GithubError, GithubSender};
use maidan_server::github_app::GithubAppAuth;
use maidan_types::{GithubCheckConclusion, GithubCheckRun};
use ring::signature::{KeyPair as _, UnparsedPublicKey, RSA_PKCS1_2048_8192_SHA256};
use serde_json::{json, Value};

mod github_app_key;

/// The app's private key, PKCS#1 PEM, as GitHub downloads it.
fn key() -> &'static str {
    &github_app_key::test_key().pkcs1_pem
}
const APP_ID: &str = "424242";
const INSTALLATION: u64 = 31337;

/// The fixture key's public half, DER, as GitHub holds it for the app.
fn public_key() -> Vec<u8> {
    ring::rsa::KeyPair::from_der(&github_app_key::test_key().pkcs1_der)
        .unwrap()
        .public_key()
        .as_ref()
        .to_vec()
}

#[derive(Clone)]
struct Fake {
    public_key: Arc<Vec<u8>>,
    /// Bearers seen on repository requests, in order.
    repo_bearers: Arc<Mutex<Vec<String>>>,
    exchanges: Arc<Mutex<u32>>,
    /// `expires_at` is this far after the exchange.
    token_life: chrono::Duration,
    /// The status the exchange answers with.
    exchange_status: StatusCode,
    /// How long the exchange takes, so concurrent callers overlap it.
    exchange_delay: Duration,
}

fn verify_jwt(fake: &Fake, jwt: &str) -> Result<(), String> {
    let parts: Vec<&str> = jwt.split('.').collect();
    if parts.len() != 3 {
        return Err(format!("not a JWT: {jwt}"));
    }
    let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let decode = |p: &str| b64.decode(p).map_err(|e| e.to_string());
    UnparsedPublicKey::new(&RSA_PKCS1_2048_8192_SHA256, fake.public_key.as_slice())
        .verify(
            format!("{}.{}", parts[0], parts[1]).as_bytes(),
            &decode(parts[2])?,
        )
        .map_err(|_| "bad signature".to_string())?;
    let header: Value = serde_json::from_slice(&decode(parts[0])?).map_err(|e| e.to_string())?;
    let claims: Value = serde_json::from_slice(&decode(parts[1])?).map_err(|e| e.to_string())?;
    let now = chrono::Utc::now().timestamp();
    let iat = claims["iat"].as_i64().unwrap_or(i64::MAX);
    let exp = claims["exp"].as_i64().unwrap_or(0);
    if header["alg"] != "RS256" {
        return Err(format!("alg {}", header["alg"]));
    }
    if claims["iss"] != json!(APP_ID.parse::<u64>().unwrap()) {
        return Err(format!("iss {}", claims["iss"]));
    }
    // GitHub's own rules: issued in the past, expiring in the future, and at
    // most ten minutes long.
    if iat > now || exp <= now || exp - iat > 600 {
        return Err(format!("iat {iat} exp {exp} now {now}"));
    }
    Ok(())
}

async fn handler(
    State(fake): State<Fake>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
) -> axum::response::Response {
    let bearer = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("")
        .to_string();
    if uri.path() == format!("/app/installations/{INSTALLATION}/access_tokens") {
        assert_eq!(method, Method::POST);
        if let Err(why) = verify_jwt(&fake, &bearer) {
            return (StatusCode::UNAUTHORIZED, Json(json!({ "message": why }))).into_response();
        }
        if fake.exchange_status != StatusCode::CREATED {
            return (fake.exchange_status, Json(json!({}))).into_response();
        }
        tokio::time::sleep(fake.exchange_delay).await;
        let n = {
            let mut exchanges = fake.exchanges.lock().unwrap();
            *exchanges += 1;
            *exchanges
        };
        let expires_at = chrono::Utc::now() + fake.token_life;
        return (
            StatusCode::CREATED,
            Json(json!({
                "token": format!("ghs_installation_{n}"),
                "expires_at": expires_at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true),
                "permissions": { "issues": "write" },
                "repository_selection": "selected",
            })),
        )
            .into_response();
    }
    if uri.path().starts_with("/app/") {
        return (StatusCode::NOT_FOUND, Json(json!({}))).into_response();
    }
    fake.repo_bearers.lock().unwrap().push(bearer);
    let body = if uri.path().contains("/git/ref/heads/") {
        json!({ "object": { "sha": "b5e54f94fd04d6ef7d6e1197ddd59ace70edb911" } })
    } else {
        json!({ "id": 7 })
    };
    (StatusCode::CREATED, Json(body)).into_response()
}

fn fake() -> Fake {
    Fake {
        public_key: Arc::new(public_key()),
        repo_bearers: Arc::default(),
        exchanges: Arc::default(),
        token_life: chrono::Duration::hours(1),
        exchange_status: StatusCode::CREATED,
        exchange_delay: Duration::ZERO,
    }
}

async fn spawn(fake: Fake) -> String {
    let app = Router::new().fallback(any(handler)).with_state(fake);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    format!("http://{addr}")
}

fn app() -> Arc<GithubAppAuth> {
    Arc::new(GithubAppAuth::new(APP_ID, &INSTALLATION.to_string(), key()).unwrap())
}

fn check_run() -> GithubCheckRun {
    GithubCheckRun {
        name: "maidan".into(),
        head_sha: "b5e54f94fd04d6ef7d6e1197ddd59ace70edb911".into(),
        conclusion: GithubCheckConclusion::Success,
        title: "t".into(),
        summary: "s".into(),
        details_url: None,
    }
}

#[tokio::test]
async fn comments_check_runs_and_git_reads_use_one_installation_token() {
    let fake = fake();
    let base = spawn(fake.clone()).await;
    let client = GithubApiClient::with_app_and_base_url(app(), base).with_any_write_repo();

    client.post_comment("acme/widgets", 1, "one").await.unwrap();
    client
        .create_check_run("acme/widgets", &check_run())
        .await
        .unwrap();
    let git = client.git().expect("the API client does git");
    assert!(git
        .branch_head("acme/widgets", "feature/agent-x")
        .await
        .unwrap()
        .is_some());

    assert_eq!(
        *fake.exchanges.lock().unwrap(),
        1,
        "a fresh token is reused, not exchanged per call"
    );
    let bearers = fake.repo_bearers.lock().unwrap().clone();
    assert_eq!(bearers, vec!["ghs_installation_1"; 3]);
}

#[tokio::test]
async fn a_token_inside_the_refresh_margin_is_exchanged_again() {
    // Four minutes left is inside the five-minute margin, so the token is
    // used once, for the call that fetched it, and never handed out again.
    let fake = Fake {
        token_life: chrono::Duration::minutes(4),
        ..fake()
    };
    let base = spawn(fake.clone()).await;
    let client = GithubApiClient::with_app_and_base_url(app(), base).with_any_write_repo();
    client.post_comment("acme/widgets", 1, "one").await.unwrap();
    client.post_comment("acme/widgets", 1, "two").await.unwrap();
    assert_eq!(*fake.exchanges.lock().unwrap(), 2);
    assert_eq!(
        fake.repo_bearers.lock().unwrap().clone(),
        vec!["ghs_installation_1", "ghs_installation_2"]
    );
}

#[tokio::test]
async fn concurrent_calls_share_one_exchange() {
    let fake = Fake {
        exchange_delay: Duration::from_millis(200),
        ..fake()
    };
    let base = spawn(fake.clone()).await;
    let client =
        Arc::new(GithubApiClient::with_app_and_base_url(app(), base).with_any_write_repo());
    let calls: Vec<_> = (0..8)
        .map(|i| {
            let client = client.clone();
            tokio::spawn(async move {
                client
                    .post_comment("acme/widgets", 1, &format!("{i}"))
                    .await
            })
        })
        .collect();
    for call in calls {
        call.await.unwrap().unwrap();
    }
    assert_eq!(*fake.exchanges.lock().unwrap(), 1);
    assert_eq!(fake.repo_bearers.lock().unwrap().len(), 8);
}

#[tokio::test]
async fn a_write_outside_the_listed_repositories_costs_no_exchange() {
    let fake = fake();
    let base = spawn(fake.clone()).await;
    let client = GithubApiClient::with_app_and_base_url(app(), base)
        .with_write_repos(["acme/widgets".to_string()]);
    let err = client.post_comment("acme/other", 1, "x").await.unwrap_err();
    assert!(matches!(err, GithubError::Refused(_)), "{err}");
    assert_eq!(*fake.exchanges.lock().unwrap(), 0);
    assert!(fake.repo_bearers.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_refused_exchange_is_a_misconfiguration_and_reaches_no_repository() {
    for status in [StatusCode::UNAUTHORIZED, StatusCode::NOT_FOUND] {
        let fake = Fake {
            exchange_status: status,
            ..fake()
        };
        let base = spawn(fake.clone()).await;
        let client = GithubApiClient::with_app_and_base_url(app(), base).with_any_write_repo();
        let err = client
            .post_comment("acme/widgets", 1, "x")
            .await
            .unwrap_err();
        // Refused, not a 404 `Api` error: callers read a 404 as a deleted
        // comment or pull request and would take a recovery path.
        assert!(matches!(err, GithubError::Refused(_)), "{err}");
        assert!(
            err.to_string().contains(&status.as_u16().to_string()),
            "{err}"
        );
        assert!(err.is_misconfiguration(), "{status}: {err}");
        assert!(!err.is_not_found() && !err.is_inline_review_skip(), "{err}");
        assert!(fake.repo_bearers.lock().unwrap().is_empty());
    }
}

#[tokio::test]
async fn a_rate_limited_or_failing_exchange_is_retried_not_disabled() {
    for status in [StatusCode::TOO_MANY_REQUESTS, StatusCode::BAD_GATEWAY] {
        let fake = Fake {
            exchange_status: status,
            ..fake()
        };
        let base = spawn(fake.clone()).await;
        let client = GithubApiClient::with_app_and_base_url(app(), base).with_any_write_repo();
        let err = client
            .post_comment("acme/widgets", 1, "x")
            .await
            .unwrap_err();
        assert!(!err.is_misconfiguration(), "{status}: {err}");
    }
}

#[tokio::test]
async fn a_jwt_signed_with_another_key_is_refused_by_github() {
    // The fake holds a different public key, as GitHub would for another
    // app: the exchange fails, which shows the fake really checks signatures.
    let mut wrong = public_key();
    let last = wrong.len() - 1;
    wrong[last / 2] ^= 0x01;
    let fake = Fake {
        public_key: Arc::new(wrong),
        ..fake()
    };
    let base = spawn(fake.clone()).await;
    let client = GithubApiClient::with_app_and_base_url(app(), base).with_any_write_repo();
    let err = client
        .post_comment("acme/widgets", 1, "x")
        .await
        .unwrap_err();
    assert!(matches!(err, GithubError::Refused(_)), "{err}");
    assert!(err.to_string().contains("401"), "{err}");
}

// ---- boot ----

fn server(dir: &Path) -> Command {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_maidan-server"));
    for (name, _) in std::env::vars() {
        if name.starts_with("MAIDAN_") || name.starts_with("DATABASE_URL") {
            cmd.env_remove(name);
        }
    }
    cmd.env_remove("AUTH_DISABLED")
        .current_dir(dir)
        .env("DATABASE_URL", "sqlite::memory:")
        .env("MAIDAN_CONTENT_KEK", "5a".repeat(32))
        .env(
            "MAIDAN_SESSION_SECRET",
            "session-secret-for-the-github-app-boot-test",
        );
    cmd
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

#[test]
fn a_partial_app_config_refuses_boot_naming_what_is_missing() {
    let dir = tempfile::tempdir().unwrap();
    let output = server(dir.path())
        .env("MAIDAN_GITHUB_APP_ID", APP_ID)
        .env("MAIDAN_GITHUB_APP_PRIVATE_KEY", key())
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !output.status.success(),
        "a partial app config must refuse boot"
    );
    assert!(
        stderr.contains("MAIDAN_GITHUB_APP_INSTALLATION_ID unset"),
        "{stderr}"
    );
    assert!(!stderr.contains("BEGIN RSA PRIVATE KEY"), "{stderr}");
}

#[test]
fn an_unusable_key_refuses_boot_without_printing_it() {
    let dir = tempfile::tempdir().unwrap();
    let output = server(dir.path())
        .env("MAIDAN_GITHUB_APP_ID", APP_ID)
        .env(
            "MAIDAN_GITHUB_APP_INSTALLATION_ID",
            INSTALLATION.to_string(),
        )
        .env(
            "MAIDAN_GITHUB_APP_PRIVATE_KEY",
            "not-a-key-but-secret-anyway",
        )
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(!output.status.success());
    assert!(
        stderr.contains("MAIDAN_GITHUB_APP_PRIVATE_KEY is not an RSA private key"),
        "{stderr}"
    );
    assert!(!stderr.contains("not-a-key-but-secret-anyway"), "{stderr}");
}

#[tokio::test]
async fn the_server_boots_with_the_app_key_from_a_file_and_never_logs_it() {
    let dir = tempfile::tempdir().unwrap();
    let key_file = dir.path().join("github_app_key.pem");
    std::fs::write(&key_file, key()).unwrap();
    let port = free_port();
    let mut child = server(dir.path())
        .env("MAIDAN_BIND", format!("127.0.0.1:{port}"))
        .env("MAIDAN_LOG", "info,sqlx=warn")
        .env("MAIDAN_GITHUB_WEBHOOK_SECRET", "whsec-for-the-boot-test")
        .env("MAIDAN_GITHUB_WRITE_REPOS", "acme/widgets")
        .env("MAIDAN_GITHUB_APP_ID", APP_ID)
        .env(
            "MAIDAN_GITHUB_APP_INSTALLATION_ID",
            INSTALLATION.to_string(),
        )
        .env("MAIDAN_GITHUB_APP_PRIVATE_KEY_FILE", &key_file)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let client = reqwest::Client::new();
    let url = format!("http://127.0.0.1:{port}/health/ready");
    let deadline = Instant::now() + Duration::from_secs(60);
    let ready = loop {
        if let Ok(response) = client.get(&url).send().await {
            if response.status().is_success() {
                break true;
            }
        }
        if Instant::now() > deadline || child.try_wait().unwrap().is_some() {
            break false;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    };
    let _ = child.kill();
    let output = child.wait_with_output().unwrap();
    let logs = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(ready, "server never became ready:\n{logs}");
    assert!(
        logs.contains("github egress authenticates as a GitHub App"),
        "{logs}"
    );
    let key_body = key().lines().nth(1).unwrap();
    assert!(!logs.contains(key_body), "the app key was logged");
}

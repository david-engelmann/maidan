//! Cluster 307: the Slack projector ingress. A correctly-signed `url_verification`
//! is echoed back its challenge; the route is `404` when the projector is not
//! configured and `401` on a bad signature.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use maidan_artifacts::LocalFsStore;
use maidan_bus::InMemoryBus;
use maidan_server::{
    router,
    slack::{slack_signature, SlackConfig},
    AppState,
};
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use reqwest::StatusCode;
use sqlx::sqlite::SqlitePoolOptions;

const SECRET: &str = "test-signing-secret";

async fn spawn(with_slack: bool) -> (SocketAddr, reqwest::Client, tokio::task::JoinHandle<()>) {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    std::mem::forget(dir); // keep the artifact dir alive for the served server (test-only)
    let bus = Arc::new(InMemoryBus::with_capacity(16));
    let mut state = AppState::for_tests(store, artifacts, bus, search);
    if with_slack {
        state.attach_slack(Arc::new(SlackConfig {
            signing_secret: SECRET.into(),
            bot_token: None,
        }));
    }
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, reqwest::Client::new(), handle)
}

fn now_ts() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        .to_string()
}

#[tokio::test]
async fn slack_url_verification_echoes_the_challenge() {
    let (addr, client, server) = spawn(true).await;
    let body = r#"{"type":"url_verification","challenge":"abc123"}"#;
    let ts = now_ts();
    let sig = slack_signature(SECRET, &ts, body);
    let resp = client
        .post(format!("http://{addr}/integrations/slack/events"))
        .header("x-slack-request-timestamp", &ts)
        .header("x-slack-signature", &sig)
        .header("content-type", "application/json")
        .body(body.to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let v: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(v["challenge"], "abc123");
    server.abort();
}

#[tokio::test]
async fn slack_ingress_is_404_when_not_configured() {
    let (addr, client, server) = spawn(false).await;
    let resp = client
        .post(format!("http://{addr}/integrations/slack/events"))
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    server.abort();
}

#[tokio::test]
async fn slack_bad_signature_is_401() {
    let (addr, client, server) = spawn(true).await;
    let ts = now_ts();
    let resp = client
        .post(format!("http://{addr}/integrations/slack/events"))
        .header("x-slack-request-timestamp", &ts)
        .header("x-slack-signature", "v0=deadbeef")
        .header("content-type", "application/json")
        .body(r#"{"type":"url_verification","challenge":"x"}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    server.abort();
}

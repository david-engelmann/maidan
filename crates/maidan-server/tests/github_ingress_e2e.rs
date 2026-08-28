//! Cluster 310: the GitHub projector ingress. A correctly-signed `ping` is ACKed;
//! the route is `404` when the projector is not configured and `401` on a bad
//! `X-Hub-Signature-256`.

use std::net::SocketAddr;
use std::sync::Arc;

use maidan_artifacts::LocalFsStore;
use maidan_bus::InMemoryBus;
use maidan_server::{github::GithubConfig, router, webhooks::sign_payload, AppState};
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use reqwest::StatusCode;
use sqlx::sqlite::SqlitePoolOptions;

const SECRET: &str = "gh-webhook-secret";

async fn spawn(with_github: bool) -> (SocketAddr, reqwest::Client, tokio::task::JoinHandle<()>) {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    std::mem::forget(dir);
    let bus = Arc::new(InMemoryBus::with_capacity(16));
    let mut state = AppState::for_tests(store, artifacts, bus, search);
    if with_github {
        state.attach_github(Arc::new(GithubConfig {
            webhook_secret: SECRET.into(),
            api_token: None,
        }));
    }
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let handle = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, reqwest::Client::new(), handle)
}

#[tokio::test]
async fn github_ping_is_acked() {
    let (addr, client, server) = spawn(true).await;
    let body = r#"{"zen":"Keep it simple."}"#;
    let sig = sign_payload(SECRET, body);
    let resp = client
        .post(format!("http://{addr}/integrations/github/events"))
        .header("x-github-event", "ping")
        .header("x-hub-signature-256", &sig)
        .header("content-type", "application/json")
        .body(body.to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    server.abort();
}

#[tokio::test]
async fn github_ingress_is_404_when_not_configured() {
    let (addr, client, server) = spawn(false).await;
    let resp = client
        .post(format!("http://{addr}/integrations/github/events"))
        .header("x-github-event", "ping")
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    server.abort();
}

#[tokio::test]
async fn github_bad_signature_is_401() {
    let (addr, client, server) = spawn(true).await;
    let resp = client
        .post(format!("http://{addr}/integrations/github/events"))
        .header("x-github-event", "ping")
        .header("x-hub-signature-256", "sha256=deadbeef")
        .header("content-type", "application/json")
        .body(r#"{"zen":"nope"}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    server.abort();
}

//! A push subscription's endpoint is an outbound target.
//!
//! The server POSTs to it on every notification, so a member who could register
//! any URL could make the server call an internal address — a blind SSRF. It is
//! held to the same boundary as webhooks: refused at registration if it names a
//! non-public address, and resolved, pinned and never redirected on delivery.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberKind, NewApiToken, NewMember, NewWorkspace};
use reqwest::StatusCode;
use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;

#[tokio::test]
async fn a_push_endpoint_must_be_a_public_https_url() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let ws = store
        .create_workspace(NewWorkspace { name: "w".into() })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "m".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: member.id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: vec![capability::WORKSPACE_READ.into()],
            expires_at: None,
        })
        .await
        .unwrap();
    let state = AppState::new(
        store,
        Arc::new(LocalFsStore::new(dir.path())),
        Arc::new(maidan_bus::InMemoryBus::new()),
        search,
        Arc::new(maidan_search::HashV1Provider),
        false, // auth ENABLED
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router(state)).await.unwrap() });
    let client = reqwest::Client::new();

    let register = |endpoint: &'static str| {
        client
            .post(format!(
                "http://{addr}/members/{}/push-subscriptions",
                member.id.0
            ))
            .bearer_auth(secret.as_str())
            .json(&json!({
                "endpoint": endpoint,
                "keys": { "p256dh": "BPk", "auth": "a2V5" }
            }))
            .send()
    };

    for (endpoint, why) in [
        ("https://127.0.0.1/push", "loopback"),
        ("https://localhost/push", "loopback by name"),
        ("https://10.0.0.5/push", "private network"),
        (
            "https://169.254.169.254/latest/meta-data/",
            "cloud metadata",
        ),
        (
            "https://user:pass@push.example.com/x",
            "embedded credentials",
        ),
        ("http://push.example.com/x", "not https"),
    ] {
        let resp = register(endpoint).await.unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::BAD_REQUEST,
            "{why}: {endpoint} must be refused"
        );
    }

    let ok = register("https://push.example.com/send/abc").await.unwrap();
    assert_eq!(ok.status(), StatusCode::OK, "{}", ok.text().await.unwrap());
}

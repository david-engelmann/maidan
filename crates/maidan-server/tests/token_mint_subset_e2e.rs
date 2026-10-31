//! A minted member token is a subset of the minting credential's grant.

use std::sync::{atomic::AtomicI64, Arc};

use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberKind, NewApiToken, NewMember, NewWorkspace};
use reqwest::StatusCode;
use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;

async fn spawn() -> (
    std::net::SocketAddr,
    reqwest::Client,
    Arc<dyn Store>,
    tokio::task::JoinHandle<()>,
) {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(maidan_artifacts::LocalFsStore::new(dir.path()));
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    let state = AppState::new(
        store.clone(),
        artifacts,
        bus,
        search,
        Arc::new(maidan_search::HashV1Provider),
        false,
        true,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, reqwest::Client::new(), store, server)
}

async fn admin(store: &Arc<dyn Store>, caps: &[&str]) -> (String, uuid::Uuid, uuid::Uuid) {
    let ws = store
        .create_workspace(NewWorkspace {
            name: "mint-ws".into(),
        })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "narrow-admin".into(),
            display_name: None,
            kind: MemberKind::Agent,
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
            capabilities: caps.iter().map(|c| (*c).to_string()).collect(),
            expires_at: None,
        })
        .await
        .unwrap();
    (format!("Bearer {}", secret.as_str()), ws.id.0, member.id.0)
}

#[tokio::test]
async fn an_escalating_mint_is_refused_and_a_subset_mint_succeeds() {
    let (addr, client, store, server) = spawn().await;
    let (auth, ws, member) = admin(
        &store,
        &[capability::WORKSPACE_READ, capability::TOKEN_ADMIN],
    )
    .await;
    let url = format!("http://{addr}/workspaces/{ws}/members/{member}/tokens");

    let escalate = client
        .post(&url)
        .header("authorization", &auth)
        .json(&json!({ "capabilities": [capability::WORKSPACE_READ, capability::WORKSPACE_WRITE] }))
        .send()
        .await
        .unwrap();
    assert_eq!(escalate.status(), StatusCode::FORBIDDEN);
    let body = escalate.text().await.unwrap();
    assert!(
        body.contains(capability::WORKSPACE_WRITE),
        "names the capability: {body}"
    );
    assert!(!body.contains("secret"), "no token in the refusal");

    // The default grant is refused too when it exceeds the minter's.
    let default = client
        .post(&url)
        .header("authorization", &auth)
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(default.status(), StatusCode::FORBIDDEN);

    let subset = client
        .post(&url)
        .header("authorization", &auth)
        .json(&json!({ "capabilities": [capability::WORKSPACE_READ] }))
        .send()
        .await
        .unwrap();
    assert_eq!(subset.status(), StatusCode::CREATED);
    let minted: serde_json::Value = subset.json().await.unwrap();
    assert_eq!(minted["capabilities"], json!([capability::WORKSPACE_READ]));
    server.abort();
}

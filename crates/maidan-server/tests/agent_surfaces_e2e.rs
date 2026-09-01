//! Agent integration surfaces (Clusters 59–67).

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret};
use maidan_server::{router, subscribe_resume, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{NewApiToken, NewWorkspace};
use reqwest::StatusCode;
use sqlx::sqlite::SqlitePoolOptions;

async fn spawn() -> (
    SocketAddr,
    reqwest::Client,
    tokio::task::JoinHandle<()>,
    Arc<dyn Store>,
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
    let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let mut state = AppState::new(
        store.clone(),
        artifacts,
        bus,
        search,
        Arc::new(maidan_search::HashV1Provider),
        true,
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    state.subscribe_resume_secret = Some(Arc::from(subscribe_resume::TEST_SUBSCRIBE_RESUME_SECRET));
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, reqwest::Client::new(), server, store)
}

#[tokio::test]
async fn well_known_and_workspace_context_respond() {
    let (addr, client, server, store) = spawn().await;
    let base = format!("http://{addr}");

    let well = client
        .get(format!("{base}/.well-known/maidan.json"))
        .send()
        .await
        .unwrap();
    assert_eq!(well.status(), StatusCode::OK);
    let body: serde_json::Value = well.json().await.unwrap();
    assert!(body.get("mcp").is_some());
    assert!(body.get("agent_card").is_some());

    let card = client
        .get(format!("{base}/.well-known/agent-card.json"))
        .send()
        .await
        .unwrap();
    assert_eq!(card.status(), StatusCode::OK);

    let ws = store
        .create_workspace(NewWorkspace {
            name: "ctx-ws".into(),
        })
        .await
        .unwrap();

    let ctx = client
        .get(format!("{base}/workspaces/{}/context", ws.id.0))
        .send()
        .await
        .unwrap();
    assert_eq!(ctx.status(), StatusCode::OK);

    server.abort();
}

#[tokio::test]
async fn mcp_tool_call_without_capability_is_rejected_when_auth_enabled() {
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
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let mut state = AppState::new(
        store.clone(),
        artifacts,
        bus,
        search,
        Arc::new(maidan_search::HashV1Provider),
        false,
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    state.subscribe_resume_secret = Some(Arc::from(subscribe_resume::TEST_SUBSCRIBE_RESUME_SECRET));
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let ws = store
        .create_workspace(NewWorkspace {
            name: "mcp-cap".into(),
        })
        .await
        .unwrap();
    let secret = maidan_auth::TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: store
                .create_member(maidan_types::NewMember {
                    workspace_id: ws.id,
                    handle: "u".into(),
                    display_name: None,
                    kind: maidan_types::MemberKind::Human,
                })
                .await
                .unwrap()
                .id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: vec![capability::WORKSPACE_READ.to_string()],
            expires_at: None,
        })
        .await
        .unwrap();

    let client = reqwest::Client::new();
    let base = format!("http://{addr}");
    let resp = client
        .post(format!("{base}/mcp"))
        .bearer_auth(secret.as_str())
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {
                "name": "post_message",
                "arguments": {}
            }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(body.get("error").is_some());

    server.abort();
}

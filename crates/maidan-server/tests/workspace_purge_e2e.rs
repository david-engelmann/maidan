//! Workspace purge HTTP + audit.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, subscribe_resume, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberKind, NewApiToken, NewMember, NewWorkspace};
use reqwest::StatusCode;
use sqlx::sqlite::SqlitePoolOptions;

async fn spawn() -> (
    SocketAddr,
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
    let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    let mut state = AppState::new(
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
    state.subscribe_resume_secret = Some(Arc::from(subscribe_resume::TEST_SUBSCRIBE_RESUME_SECRET));
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::new();
    (addr, client, store, server)
}

#[tokio::test]
async fn purge_workspace_requires_authority_and_writes_audit() {
    let (addr, client, store, server) = spawn().await;
    let base = format!("http://{addr}");

    let ws = store
        .create_workspace(NewWorkspace {
            name: "purge-http".into(),
        })
        .await
        .unwrap();
    let alice = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "alice".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: alice.id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: Some("read-only".into()),
            capabilities: vec![capability::WORKSPACE_READ.into()],
            expires_at: None,
        })
        .await
        .unwrap();

    let denied = client
        .post(format!("{base}/workspaces/{}/purge", ws.id.0))
        .header("authorization", format!("Bearer {}", secret.as_str()))
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::FORBIDDEN);

    // Every capability an ordinary agent is minted with (`maidan.agent.worker`).
    // It used to be enough to destroy the workspace's content, because purge
    // was gated on `workspace:write` — work, which every agent holds and any
    // delegation grant can lend.
    let agent_secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: alice.id,
            app_installation_id: None,
            token_hash: hash_secret(agent_secret.as_str()),
            label: Some("agent".into()),
            capabilities: vec![
                capability::WORKSPACE_READ.into(),
                capability::WORKSPACE_WRITE.into(),
                capability::MESSAGE_POST.into(),
                capability::EVENT_SUBSCRIBE.into(),
                capability::SEARCH_QUERY.into(),
                capability::ARTIFACT_UPLOAD.into(),
                capability::THREAD_TRANSITION.into(),
            ],
            expires_at: None,
        })
        .await
        .unwrap();
    let agent = client
        .post(format!("{base}/workspaces/{}/purge", ws.id.0))
        .header("authorization", format!("Bearer {}", agent_secret.as_str()))
        .send()
        .await
        .unwrap();
    assert_eq!(
        agent.status(),
        StatusCode::FORBIDDEN,
        "an ordinary agent must not be able to purge the workspace"
    );

    let admin_secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: alice.id,
            app_installation_id: None,
            token_hash: hash_secret(admin_secret.as_str()),
            label: None,
            capabilities: vec![capability::TOKEN_ADMIN.into()],
            expires_at: None,
        })
        .await
        .unwrap();

    let ok = client
        .post(format!("{base}/workspaces/{}/purge", ws.id.0))
        .header("authorization", format!("Bearer {}", admin_secret.as_str()))
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status(), StatusCode::OK);
    let body: serde_json::Value = ok.json().await.unwrap();
    assert_eq!(body["workspace_id"], ws.id.0.to_string());

    let audit = store.list_audit(10).await.unwrap();
    assert!(audit.iter().any(|e| e.action == "workspace.purge"));

    server.abort();
}

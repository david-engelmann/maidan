//! Bearer auth and capability enforcement (SQLite harness, auth enabled).

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::{MemberKind, NewApiToken, NewMember, NewWorkspace};
use reqwest::StatusCode;
use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;

struct Harness {
    addr: SocketAddr,
    server: tokio::task::JoinHandle<()>,
    client: reqwest::Client,
    store: Arc<dyn Store>,
    _dir: tempfile::TempDir,
}

impl Harness {
    fn base(&self) -> String {
        format!("http://{}", self.addr)
    }

    async fn shutdown(self) {
        self.server.abort();
    }
}

async fn spawn() -> Harness {
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
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
    let app = router(AppState::new(
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
    ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    Harness {
        addr,
        server,
        client,
        store,
        _dir: dir,
    }
}

async fn seed_workspace(store: &dyn Store) -> (maidan_types::WorkspaceId, maidan_types::MemberId) {
    let ws = store
        .create_workspace(NewWorkspace {
            name: "auth-ws".to_string(),
        })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "agent".to_string(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    (ws.id, member.id)
}

async fn mint_token(
    store: &dyn Store,
    workspace_id: maidan_types::WorkspaceId,
    member_id: maidan_types::MemberId,
    capabilities: Vec<String>,
) -> String {
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id,
            member_id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities,
            expires_at: None,
        })
        .await
        .unwrap();
    secret.as_str().to_string()
}

async fn seed_thread(h: &Harness, bearer: &str, workspace_id: &str) -> (String, String) {
    let base = h.base();
    let ch: serde_json::Value = h
        .client
        .post(format!("{base}/workspaces/{workspace_id}/channels"))
        .header("Authorization", format!("Bearer {bearer}"))
        .json(&json!({"name": "general"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let channel_id = ch["id"].as_str().unwrap();
    let th: serde_json::Value = h
        .client
        .post(format!("{base}/channels/{channel_id}/threads"))
        .header("Authorization", format!("Bearer {bearer}"))
        .json(&json!({}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    (
        channel_id.to_string(),
        th["id"].as_str().unwrap().to_string(),
    )
}

#[tokio::test]
async fn unauthenticated_get_workspace_returns_401() {
    let h = spawn().await;
    let (workspace_id, _) = seed_workspace(h.store.as_ref()).await;
    let resp = h
        .client
        .get(format!("{}/workspaces/{}", h.base(), workspace_id.0))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    h.shutdown().await;
}

#[tokio::test]
async fn post_message_without_capability_returns_403() {
    let h = spawn().await;
    let (workspace_id, member_id) = seed_workspace(h.store.as_ref()).await;
    let bearer = mint_token(
        h.store.as_ref(),
        workspace_id,
        member_id,
        vec![
            capability::WORKSPACE_READ.to_string(),
            capability::WORKSPACE_WRITE.to_string(),
        ],
    )
    .await;
    let ws = workspace_id.0.to_string();
    let member = member_id.0.to_string();
    let (_, thread_id) = seed_thread(&h, &bearer, &ws).await;

    let resp = h
        .client
        .post(format!("{}/threads/{thread_id}/messages", h.base()))
        .header("Authorization", format!("Bearer {bearer}"))
        .json(&json!({"author_id": member, "body": "hi"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    h.shutdown().await;
}

#[tokio::test]
async fn post_message_with_message_post_succeeds() {
    let h = spawn().await;
    let (workspace_id, member_id) = seed_workspace(h.store.as_ref()).await;
    let bearer = mint_token(
        h.store.as_ref(),
        workspace_id,
        member_id,
        vec![
            capability::WORKSPACE_READ.to_string(),
            capability::WORKSPACE_WRITE.to_string(),
            capability::MESSAGE_POST.to_string(),
        ],
    )
    .await;
    let ws = workspace_id.0.to_string();
    let member = member_id.0.to_string();
    let (_, thread_id) = seed_thread(&h, &bearer, &ws).await;

    let resp = h
        .client
        .post(format!("{}/threads/{thread_id}/messages", h.base()))
        .header("Authorization", format!("Bearer {bearer}"))
        .json(&json!({"author_id": member, "body": "hi"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    h.shutdown().await;
}

#[tokio::test]
async fn mint_and_revoke_token_roundtrip() {
    let h = spawn().await;
    let (workspace_id, member_id) = seed_workspace(h.store.as_ref()).await;
    let admin = mint_token(
        h.store.as_ref(),
        workspace_id,
        member_id,
        vec![capability::TOKEN_ADMIN.to_string()],
    )
    .await;
    let ws = workspace_id.0.to_string();
    let mid = member_id.0.to_string();

    let minted: serde_json::Value = h
        .client
        .post(format!("{}/workspaces/{ws}/members/{mid}/tokens", h.base()))
        .header("Authorization", format!("Bearer {admin}"))
        .json(&json!({"label": "bot"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let secret = minted["secret"].as_str().unwrap();
    let token_id = minted["id"].as_str().unwrap();

    let ok = h
        .client
        .get(format!("{}/workspaces/{ws}", h.base()))
        .header("Authorization", format!("Bearer {secret}"))
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status(), StatusCode::OK);

    let revoked = h
        .client
        .delete(format!("{}/tokens/{token_id}", h.base()))
        .header("Authorization", format!("Bearer {admin}"))
        .send()
        .await
        .unwrap();
    assert_eq!(revoked.status(), StatusCode::OK);

    let denied = h
        .client
        .get(format!("{}/workspaces/{ws}", h.base()))
        .header("Authorization", format!("Bearer {secret}"))
        .send()
        .await
        .unwrap();
    assert_eq!(denied.status(), StatusCode::UNAUTHORIZED);
    h.shutdown().await;
}

#[tokio::test]
async fn list_api_tokens_returns_metadata_without_secret() {
    let h = spawn().await;
    let (workspace_id, member_id) = seed_workspace(h.store.as_ref()).await;
    let admin = mint_token(
        h.store.as_ref(),
        workspace_id,
        member_id,
        vec![capability::TOKEN_ADMIN.to_string()],
    )
    .await;
    let ws = workspace_id.0.to_string();
    let mid = member_id.0.to_string();

    let minted: serde_json::Value = h
        .client
        .post(format!("{}/workspaces/{ws}/members/{mid}/tokens", h.base()))
        .header("Authorization", format!("Bearer {admin}"))
        .json(&json!({"label": "listed"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let token_id = minted["id"].as_str().unwrap();

    let listed: Vec<serde_json::Value> = h
        .client
        .get(format!("{}/workspaces/{ws}/members/{mid}/tokens", h.base()))
        .header("Authorization", format!("Bearer {admin}"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let row = listed
        .iter()
        .find(|t| t["id"].as_str() == Some(token_id))
        .expect("minted token in list response");
    assert_eq!(row["label"].as_str().unwrap(), "listed");
    for t in &listed {
        assert!(t.get("secret").is_none());
    }
    h.shutdown().await;
}

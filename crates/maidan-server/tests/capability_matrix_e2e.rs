//! Capability denial matrix: HTTP, MCP JSON-RPC, and A2A JSON-RPC with auth enabled.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use futures::{SinkExt, StreamExt};
use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, subscribe_resume, AppState, FederationRuntime};
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::{MemberKind, NewApiToken, NewMember, NewWorkspace};
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{
        client::IntoClientRequest,
        protocol::frame::coding::CloseCode,
        Message,
    },
};

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
            name: "cap-ws".to_string(),
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
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities,
            expires_at: None,
        })
        .await
        .unwrap();
    secret.as_str().to_string()
}

async fn seed_thread(h: &Harness, bearer: &str, workspace_id: &str) -> String {
    let base = h.base();
    let ch: Value = h
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
    let th: Value = h
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
    th["id"].as_str().unwrap().to_string()
}

async fn mcp_rpc(
    h: &Harness,
    bearer: &str,
    method: &str,
    params: Value,
) -> Value {
    h.client
        .post(format!("{}/mcp", h.base()))
        .header("Authorization", format!("Bearer {bearer}"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
            "params": params,
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

fn read_only_caps() -> Vec<String> {
    vec![
        capability::WORKSPACE_READ.to_string(),
        capability::WORKSPACE_WRITE.to_string(),
    ]
}

#[tokio::test]
async fn search_without_search_query_returns_403() {
    let h = spawn().await;
    let (workspace_id, member_id) = seed_workspace(h.store.as_ref()).await;
    let bearer = mint_token(
        h.store.as_ref(),
        workspace_id,
        member_id,
        read_only_caps(),
    )
    .await;
    let ws = workspace_id.0.to_string();
    let resp = h
        .client
        .get(format!("{}/workspaces/{ws}/search", h.base()))
        .query(&[("q", "hello")])
        .header("Authorization", format!("Bearer {bearer}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    h.shutdown().await;
}

#[tokio::test]
async fn mcp_post_message_without_message_post_returns_forbidden_jsonrpc() {
    let h = spawn().await;
    let (workspace_id, member_id) = seed_workspace(h.store.as_ref()).await;
    let bearer = mint_token(
        h.store.as_ref(),
        workspace_id,
        member_id,
        read_only_caps(),
    )
    .await;
    let ws = workspace_id.0.to_string();
    let thread_id = seed_thread(&h, &bearer, &ws).await;
    let member = member_id.0.to_string();

    let resp = mcp_rpc(
        &h,
        &bearer,
        "tools/call",
        json!({
            "name": "post_message",
            "arguments": {
                "thread_id": thread_id,
                "author_id": member,
                "body": "nope"
            }
        }),
    )
    .await;
    assert_eq!(resp["error"]["code"], -32003);
    h.shutdown().await;
}

#[tokio::test]
async fn a2a_send_message_without_message_post_returns_jsonrpc_forbidden() {
    let h = spawn().await;
    let (workspace_id, member_id) = seed_workspace(h.store.as_ref()).await;
    let bearer = mint_token(
        h.store.as_ref(),
        workspace_id,
        member_id,
        read_only_caps(),
    )
    .await;
    let ws = workspace_id.0.to_string();
    let thread_id = seed_thread(&h, &bearer, &ws).await;

    let resp: Value = h
        .client
        .post(format!("{}/a2a/v1/rpc", h.base()))
        .header("Authorization", format!("Bearer {bearer}"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "SendMessage",
            "params": {
                "message": {
                    "role": "user",
                    "parts": [{ "type": "text", "text": "hi" }]
                },
                "metadata": {
                    "maidan": { "threadId": thread_id, "authorId": member_id.0 }
                }
            }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(resp["error"]["code"], -32001);
    h.shutdown().await;
}

#[tokio::test]
async fn upload_artifact_without_artifact_upload_returns_403() {
    let h = spawn().await;
    let (workspace_id, member_id) = seed_workspace(h.store.as_ref()).await;
    let bearer = mint_token(
        h.store.as_ref(),
        workspace_id,
        member_id,
        read_only_caps(),
    )
    .await;
    let resp = h
        .client
        .post(format!("{}/artifacts", h.base()))
        .query(&[("kind", "attachment")])
        .header("Authorization", format!("Bearer {bearer}"))
        .body("bytes")
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    h.shutdown().await;
}

#[tokio::test]
async fn ws_subscribe_without_event_subscribe_closes_with_policy_violation() {
    let h = spawn().await;
    let (workspace_id, member_id) = seed_workspace(h.store.as_ref()).await;
    let bearer = mint_token(
        h.store.as_ref(),
        workspace_id,
        member_id,
        read_only_caps(),
    )
    .await;
    let ws_url = format!("ws://{}/ws/subscribe", h.addr);
    let (mut ws, _) = connect_async(ws_url.into_client_request().unwrap())
        .await
        .expect("ws connect");
    ws.send(Message::Text(
        json!({
            "filter": { "workspace_id": workspace_id.0 },
            "token": bearer
        })
        .to_string(),
    ))
    .await
    .unwrap();
    let close = ws.next().await.expect("close frame");
    match close {
        Ok(Message::Close(Some(frame))) => assert_eq!(frame.code, CloseCode::Policy),
        other => panic!("expected close 1008, got {other:?}"),
    }
    h.shutdown().await;
}

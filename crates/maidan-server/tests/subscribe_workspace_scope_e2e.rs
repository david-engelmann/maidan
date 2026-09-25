//! A live subscription is bound to its token's workspace.
//!
//! `/ws/subscribe`, `/mcp/stream` and `/agui/stream` checked the workspace only
//! when the subscriber named one. A filter without `workspace_id` matched every
//! event on the bus, so any token with `event:subscribe` (or a browser session)
//! received every tenant's live events — messages, DMs and private-channel
//! traffic included — and the per-workspace private-channel grants were skipped
//! too. An omitted workspace now means the caller's own.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use futures::{SinkExt, StreamExt};
use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, subscribe_resume, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberKind, NewApiToken, NewMember, NewWorkspace, WorkspaceId};
use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message},
};

struct Harness {
    addr: SocketAddr,
    client: reqwest::Client,
    store: Arc<dyn Store>,
    server: tokio::task::JoinHandle<()>,
    _dir: tempfile::TempDir,
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
    let mut state = AppState::new(
        store.clone(),
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
    state.subscribe_resume_secret = Some(Arc::from(subscribe_resume::TEST_SUBSCRIBE_RESUME_SECRET));
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Harness {
        addr,
        client: reqwest::Client::builder()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap(),
        store,
        server,
        _dir: dir,
    }
}

/// A workspace and a token for one of its members.
async fn tenant(store: &dyn Store, name: &str) -> (WorkspaceId, String) {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: format!("{name}-agent"),
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
            capabilities: vec![
                capability::WORKSPACE_READ.into(),
                capability::WORKSPACE_WRITE.into(),
                capability::MESSAGE_POST.into(),
                capability::EVENT_SUBSCRIBE.into(),
            ],
            expires_at: None,
        })
        .await
        .unwrap();
    (ws.id, secret.as_str().to_string())
}

/// Create a channel whose name marks the tenant; its `ChannelCreated` event
/// is what a leak would carry.
async fn announce(h: &Harness, ws: WorkspaceId, bearer: &str, marker: &str) {
    let resp = h
        .client
        .post(format!("http://{}/workspaces/{}/channels", h.addr, ws.0))
        .header("Authorization", format!("Bearer {bearer}"))
        .json(&json!({ "name": marker }))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success(), "{}", resp.status());
}

/// Every frame received within `window`.
async fn drain_ws<S>(ws: &mut S, window: Duration) -> Vec<String>
where
    S: futures::Stream<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    let mut frames = Vec::new();
    let deadline = tokio::time::Instant::now() + window;
    while let Ok(Some(Ok(msg))) = tokio::time::timeout_at(deadline, ws.next()).await {
        if let Message::Text(text) = msg {
            frames.push(text.to_string());
        }
    }
    frames
}

#[tokio::test]
async fn a_websocket_subscription_without_a_workspace_is_the_callers_own() {
    let h = spawn().await;
    let (ws_a, token_a) = tenant(h.store.as_ref(), "alpha").await;
    let (ws_b, token_b) = tenant(h.store.as_ref(), "bravo").await;

    let url = format!("ws://{}/ws/subscribe", h.addr);
    let (mut ws, _) = connect_async(url.into_client_request().unwrap())
        .await
        .unwrap();
    ws.send(Message::Text(
        json!({ "token": token_b, "filter": {} }).to_string(),
    ))
    .await
    .unwrap();
    // Let the subscription register before anything is published.
    drain_ws(&mut ws, Duration::from_millis(300)).await;

    announce(&h, ws_a, &token_a, "alpha-secret-channel").await;
    announce(&h, ws_b, &token_b, "bravo-own-channel").await;
    let frames = drain_ws(&mut ws, Duration::from_millis(800))
        .await
        .join("\n");

    assert!(
        frames.contains("bravo-own-channel"),
        "the subscriber's own events still arrive: {frames}"
    );
    assert!(
        !frames.contains("alpha-secret-channel"),
        "another tenant's event reached this subscriber: {frames}"
    );
    h.server.abort();
}

#[tokio::test]
async fn an_mcp_event_stream_without_a_workspace_is_the_callers_own() {
    let h = spawn().await;
    let (ws_a, token_a) = tenant(h.store.as_ref(), "alpha").await;
    let (ws_b, token_b) = tenant(h.store.as_ref(), "bravo").await;

    let resp = h
        .client
        .get(format!("http://{}/mcp/stream", h.addr))
        .header("Authorization", format!("Bearer {token_b}"))
        .timeout(Duration::from_secs(3))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success(), "{}", resp.status());
    let mut body = resp.bytes_stream();
    tokio::time::sleep(Duration::from_millis(300)).await;

    announce(&h, ws_a, &token_a, "alpha-secret-channel").await;
    announce(&h, ws_b, &token_b, "bravo-own-channel").await;
    let mut seen = String::new();
    let deadline = tokio::time::Instant::now() + Duration::from_millis(1200);
    while let Ok(Some(Ok(chunk))) = tokio::time::timeout_at(deadline, body.next()).await {
        seen.push_str(&String::from_utf8_lossy(&chunk));
    }

    assert!(
        seen.contains("bravo-own-channel"),
        "the subscriber's own events still arrive: {seen}"
    );
    assert!(
        !seen.contains("alpha-secret-channel"),
        "another tenant's event reached this subscriber: {seen}"
    );
    h.server.abort();
}

/// A named workspace that is not the caller's is still refused.
#[tokio::test]
async fn a_subscription_naming_another_workspace_is_refused() {
    let h = spawn().await;
    let (ws_a, _) = tenant(h.store.as_ref(), "alpha").await;
    let (_, token_b) = tenant(h.store.as_ref(), "bravo").await;
    let resp = h
        .client
        .get(format!(
            "http://{}/mcp/stream?workspace_id={}",
            h.addr, ws_a.0
        ))
        .header("Authorization", format!("Bearer {token_b}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::FORBIDDEN);
    h.server.abort();
}

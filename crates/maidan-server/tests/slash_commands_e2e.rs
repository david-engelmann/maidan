//! Slash command registration and dispatch (Cluster 51.0).

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use axum::{body::Bytes, extract::State, http::HeaderMap, routing::post, Router};
use maidan_bus::InMemoryBus;
use maidan_search::{Indexer, LoggingHandler};
use maidan_server::{router, AppState, SlashRuntime, WebhookRuntime};
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;
use tokio::sync::Mutex;

fn test_key() -> Option<Arc<[u8; 32]>> {
    Some(Arc::new([0x51; 32]))
}

#[derive(Clone)]
struct HandlerState {
    secret: Arc<Mutex<String>>,
    last_text: Arc<Mutex<Option<String>>>,
}

async fn slash_http_handler(
    State(state): State<HandlerState>,
    headers: HeaderMap,
    body: Bytes,
) -> axum::Json<serde_json::Value> {
    let body_str = String::from_utf8_lossy(&body).into_owned();
    let signature = headers
        .get("X-Maidan-Signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !maidan_server::webhooks::verify_signature(&state.secret.lock().await, &body_str, signature)
    {
        return axum::Json(json!({ "error": "bad signature" }));
    }
    let payload: serde_json::Value = serde_json::from_str(&body_str).unwrap_or(json!({}));
    let text = payload
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    *state.last_text.lock().await = Some(text.clone());
    axum::Json(json!({ "text": format!("echo:{text}") }))
}

struct Harness {
    base: String,
    handler_addr: SocketAddr,
    server: tokio::task::JoinHandle<()>,
    handler: tokio::task::JoinHandle<()>,
    indexer: maidan_search::IndexerHandle,
    client: reqwest::Client,
    handler_state: HandlerState,
    _dir: tempfile::TempDir,
}

impl Harness {
    async fn shutdown(self) {
        self.indexer.shutdown().await;
        self.server.abort();
        self.handler.abort();
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
    let artifacts = Arc::new(maidan_artifacts::LocalFsStore::new(dir.path()));
    let bus = Arc::new(InMemoryBus::new());
    let mut state = AppState::new(
        store,
        artifacts,
        bus.clone(),
        search,
        Arc::new(maidan_search::HashV1Provider),
        true,
        true,
        maidan_server::FederationRuntime::new(true, test_key()),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    state.webhooks = WebhookRuntime::new(test_key());
    state.slash = SlashRuntime::new(test_key());
    let indexer = Indexer::new(bus, Arc::new(LoggingHandler::default()))
        .spawn_with_heartbeat(state.indexer_last_event_unix_ms.clone());
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let handler_state = HandlerState {
        secret: Arc::new(Mutex::new(String::new())),
        last_text: Arc::new(Mutex::new(None)),
    };
    let handler_app = Router::new()
        .route("/slash", post(slash_http_handler))
        .with_state(handler_state.clone());
    let handler_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let handler_addr = handler_listener.local_addr().unwrap();
    let handler =
        tokio::spawn(async move { axum::serve(handler_listener, handler_app).await.unwrap() });

    Harness {
        base: format!("http://{addr}"),
        handler_addr,
        server,
        handler,
        indexer,
        client: reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap(),
        handler_state,
        _dir: dir,
    }
}

async fn bootstrap_member(h: &Harness, wid: &str) -> String {
    let member: serde_json::Value = h
        .client
        .post(format!("{}/workspaces/{wid}/members", h.base))
        .json(&json!({ "handle": "author", "kind": "agent" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    member["id"].as_str().unwrap().to_string()
}

async fn setup_thread(h: &Harness) -> (String, String, String) {
    let ws: serde_json::Value = h
        .client
        .post(format!("{}/workspaces", h.base))
        .json(&json!({ "name": "slash" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let wid = ws["id"].as_str().unwrap().to_string();
    let author_id = bootstrap_member(h, &wid).await;
    let ch: serde_json::Value = h
        .client
        .post(format!("{}/workspaces/{wid}/channels", h.base))
        .json(&json!({ "name": "general" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let cid = ch["id"].as_str().unwrap().to_string();
    let th: serde_json::Value = h
        .client
        .post(format!("{}/channels/{cid}/threads", h.base))
        .json(&json!({ "title": "t" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let tid = th["id"].as_str().unwrap().to_string();
    (wid, author_id, tid)
}

#[tokio::test]
async fn http_slash_command_dispatches_signed_handler_and_stores_response() {
    let h = spawn().await;
    let (wid, author_id, tid) = setup_thread(&h).await;

    let handler_url = format!("http://{}/slash", h.handler_addr);
    let mint: serde_json::Value = h
        .client
        .post(format!("{}/workspaces/{wid}/slash-commands", h.base))
        .json(&json!({
            "name": "ping",
            "handler_kind": "http",
            "handler_target": handler_url
        }))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    *h.handler_state.secret.lock().await = mint["secret"].as_str().unwrap().to_string();

    let msg: serde_json::Value = h
        .client
        .post(format!("{}/threads/{tid}/messages", h.base))
        .json(&json!({ "author_id": author_id, "body": "/ping hello" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(msg["metadata"]["slash_command"]["name"], "ping");
    assert_eq!(msg["metadata"]["slash_response"]["ok"], true);
    assert!(msg["metadata"]["slash_response"]["response"]["text"]
        .as_str()
        .unwrap()
        .contains("echo:hello"));
    assert_eq!(
        h.handler_state.last_text.lock().await.as_deref(),
        Some("hello")
    );

    h.shutdown().await;
}

#[tokio::test]
async fn mcp_tool_slash_command_lists_channels() {
    let h = spawn().await;
    let (wid, author_id, tid) = setup_thread(&h).await;

    let _reg: serde_json::Value = h
        .client
        .post(format!("{}/workspaces/{wid}/slash-commands", h.base))
        .json(&json!({
            "name": "channels",
            "handler_kind": "mcp_tool",
            "handler_target": "list_channels"
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let msg: serde_json::Value = h
        .client
        .post(format!("{}/threads/{tid}/messages", h.base))
        .json(&json!({ "author_id": author_id, "body": "/channels" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert_eq!(msg["metadata"]["slash_response"]["ok"], true);
    let response_text = msg["metadata"]["slash_response"]["response"]["content"][0]["text"]
        .as_str()
        .unwrap();
    assert!(response_text.contains("general"));

    h.shutdown().await;
}

#[tokio::test]
async fn unregistered_slash_body_posts_without_slash_metadata() {
    let h = spawn().await;
    let (_wid, author_id, tid) = setup_thread(&h).await;

    let msg: serde_json::Value = h
        .client
        .post(format!("{}/threads/{tid}/messages", h.base))
        .json(&json!({ "author_id": author_id, "body": "/unknown arg" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert!(msg.get("metadata").is_none() || msg["metadata"].get("slash_command").is_none());

    h.shutdown().await;
}

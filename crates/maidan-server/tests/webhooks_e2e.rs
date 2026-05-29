//! Outbound webhook delivery with HMAC verification (Cluster 50.0).

use std::{
    net::SocketAddr,
    sync::{
        atomic::{AtomicBool, AtomicI64, Ordering},
        Arc,
    },
    time::Duration,
};

use axum::{
    body::Bytes,
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::post,
    Router,
};
use maidan_bus::InMemoryBus;
use maidan_search::{Indexer, LoggingHandler};
use maidan_server::{router, webhook_worker::WebhookWorker, AppState, WebhookRuntime};
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;
use tokio::sync::Mutex;

fn webhook_test_key() -> Option<Arc<[u8; 32]>> {
    Some(Arc::new([0x55; 32]))
}

#[derive(Clone)]
struct ReceiverState {
    secret: Arc<Mutex<String>>,
    received: Arc<AtomicBool>,
    last_body: Arc<Mutex<Option<String>>>,
    last_signature: Arc<Mutex<Option<String>>>,
}

async fn receiver_handler(
    State(state): State<ReceiverState>,
    headers: HeaderMap,
    body: Bytes,
) -> StatusCode {
    let body_str = String::from_utf8_lossy(&body).into_owned();
    let signature = headers
        .get("X-Maidan-Signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    if !maidan_server::webhooks::verify_signature(&state.secret.lock().await, &body_str, &signature)
    {
        return StatusCode::UNAUTHORIZED;
    }
    *state.last_body.lock().await = Some(body_str);
    *state.last_signature.lock().await = Some(signature);
    state.received.store(true, Ordering::SeqCst);
    StatusCode::OK
}

struct Harness {
    base: String,
    receiver_addr: SocketAddr,
    server: tokio::task::JoinHandle<()>,
    receiver: tokio::task::JoinHandle<()>,
    webhook_worker: WebhookWorker,
    indexer: maidan_search::IndexerHandle,
    client: reqwest::Client,
    receiver_state: ReceiverState,
    _dir: tempfile::TempDir,
}

impl Harness {
    async fn shutdown(self) {
        self.webhook_worker.shutdown().await;
        self.indexer.shutdown().await;
        self.server.abort();
        self.receiver.abort();
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
        maidan_server::FederationRuntime::new(true, webhook_test_key()),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    state.webhooks = WebhookRuntime::new(webhook_test_key());
    let indexer = Indexer::new(bus.clone(), Arc::new(LoggingHandler::default()))
        .spawn_with_heartbeat(state.indexer_last_event_unix_ms.clone());
    let webhook_worker = WebhookWorker::spawn(state.clone());
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let receiver_state = ReceiverState {
        secret: Arc::new(Mutex::new(String::new())),
        received: Arc::new(AtomicBool::new(false)),
        last_body: Arc::new(Mutex::new(None)),
        last_signature: Arc::new(Mutex::new(None)),
    };
    let recv_app = Router::new()
        .route("/hook", post(receiver_handler))
        .with_state(receiver_state.clone());
    let recv_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let receiver_addr = recv_listener.local_addr().unwrap();
    let receiver = tokio::spawn(async move { axum::serve(recv_listener, recv_app).await.unwrap() });

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();

    Harness {
        base: format!("http://{addr}"),
        receiver_addr,
        server,
        receiver,
        webhook_worker,
        indexer,
        client,
        receiver_state,
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

async fn wait_for_received(state: &ReceiverState, timeout: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    while tokio::time::Instant::now() < deadline {
        if state.received.load(Ordering::SeqCst) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    false
}

#[tokio::test]
async fn webhook_delivers_signed_event_on_message_posted() {
    let h = spawn().await;

    let ws: serde_json::Value = h
        .client
        .post(format!("{}/workspaces", h.base))
        .json(&json!({ "name": "hooks" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let wid = ws["id"].as_str().unwrap();
    let author_id = bootstrap_member(&h, wid).await;

    let hook_url = format!("http://{}/hook", h.receiver_addr);
    let mint: serde_json::Value = h
        .client
        .post(format!("{}/workspaces/{wid}/webhooks", h.base))
        .json(&json!({
            "url": hook_url,
            "label": "test",
            "event_kinds": ["message_posted"]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let secret = mint["secret"].as_str().unwrap().to_string();
    *h.receiver_state.secret.lock().await = secret;

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
    let cid = ch["id"].as_str().unwrap();

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
    let tid = th["id"].as_str().unwrap();

    let msg = h
        .client
        .post(format!("{}/threads/{tid}/messages", h.base))
        .json(&json!({ "author_id": author_id, "body": "hello webhook" }))
        .send()
        .await
        .unwrap();
    assert_eq!(msg.status(), StatusCode::CREATED);

    assert!(
        wait_for_received(&h.receiver_state, Duration::from_secs(5)).await,
        "webhook receiver did not get POST"
    );
    let body = h.receiver_state.last_body.lock().await.clone().unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(parsed["kind"], "message_posted");
    assert_eq!(parsed["event"]["kind"], "message_posted");

    h.shutdown().await;
}

#[tokio::test]
async fn webhook_filters_by_event_kind() {
    let h = spawn().await;

    let ws: serde_json::Value = h
        .client
        .post(format!("{}/workspaces", h.base))
        .json(&json!({ "name": "filter" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let wid = ws["id"].as_str().unwrap();
    let _author_id = bootstrap_member(&h, wid).await;

    let hook_url = format!("http://{}/hook", h.receiver_addr);
    let mint: serde_json::Value = h
        .client
        .post(format!("{}/workspaces/{wid}/webhooks", h.base))
        .json(&json!({
            "url": hook_url,
            "event_kinds": ["thread_created"]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    *h.receiver_state.secret.lock().await = mint["secret"].as_str().unwrap().to_string();

    let ch: serde_json::Value = h
        .client
        .post(format!("{}/workspaces/{wid}/channels", h.base))
        .json(&json!({ "name": "c" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let cid = ch["id"].as_str().unwrap();

    let _ = h
        .client
        .post(format!("{}/channels/{cid}/threads", h.base))
        .json(&json!({ "title": "only-thread" }))
        .send()
        .await
        .unwrap();

    assert!(
        wait_for_received(&h.receiver_state, Duration::from_secs(5)).await,
        "expected thread_created webhook"
    );

    h.shutdown().await;
}

#[tokio::test]
async fn revoke_webhook_stops_delivery() {
    let h = spawn().await;

    let ws: serde_json::Value = h
        .client
        .post(format!("{}/workspaces", h.base))
        .json(&json!({ "name": "revoke" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let wid = ws["id"].as_str().unwrap();
    let author_id = bootstrap_member(&h, wid).await;

    let hook_url = format!("http://{}/hook", h.receiver_addr);
    let mint: serde_json::Value = h
        .client
        .post(format!("{}/workspaces/{wid}/webhooks", h.base))
        .json(&json!({
            "url": hook_url,
            "event_kinds": ["message_posted"]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    *h.receiver_state.secret.lock().await = mint["secret"].as_str().unwrap().to_string();
    let whid = mint["webhook"]["id"].as_str().unwrap();

    let ch: serde_json::Value = h
        .client
        .post(format!("{}/workspaces/{wid}/channels", h.base))
        .json(&json!({ "name": "c" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let cid = ch["id"].as_str().unwrap();
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
    let tid = th["id"].as_str().unwrap();

    let _ = h
        .client
        .post(format!("{}/threads/{tid}/messages", h.base))
        .json(&json!({ "author_id": author_id, "body": "first" }))
        .send()
        .await
        .unwrap();
    assert!(wait_for_received(&h.receiver_state, Duration::from_secs(5)).await);

    h.receiver_state.received.store(false, Ordering::SeqCst);
    let del = h
        .client
        .delete(format!("{}/workspaces/{wid}/webhooks/{whid}", h.base))
        .send()
        .await
        .unwrap();
    assert_eq!(del.status(), StatusCode::NO_CONTENT);

    let _ = h
        .client
        .post(format!("{}/threads/{tid}/messages", h.base))
        .json(&json!({ "author_id": author_id, "body": "second" }))
        .send()
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        !h.receiver_state.received.load(Ordering::SeqCst),
        "revoked webhook should not deliver"
    );

    h.shutdown().await;
}

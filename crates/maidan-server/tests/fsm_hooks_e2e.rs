//! FSM hooks fire on ThreadStateChanged via the event bus.

use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use axum::{body::Bytes, extract::State, http::HeaderMap, routing::post, Router};
use maidan_bus::InMemoryBus;
use maidan_search::{Indexer, LoggingHandler};
use maidan_server::{router, AppState, FsmHookRuntime, SlashRuntime, WebhookRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;
use tokio::sync::Mutex;

fn test_key() -> Option<Arc<[u8; 32]>> {
    Some(Arc::new([0x52; 32]))
}

#[derive(Clone)]
struct HandlerState {
    secret: Arc<Mutex<String>>,
    received: Arc<AtomicBool>,
    last_from: Arc<Mutex<Option<String>>>,
    last_to: Arc<Mutex<Option<String>>>,
}

async fn fsm_http_handler(
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
    *state.last_from.lock().await = payload
        .get("from_state")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    *state.last_to.lock().await = payload
        .get("to_state")
        .and_then(|v| v.as_str())
        .map(str::to_string);
    state.received.store(true, Ordering::SeqCst);
    axum::Json(json!({ "ok": true }))
}

struct Harness {
    base: String,
    handler_addr: std::net::SocketAddr,
    server: tokio::task::JoinHandle<()>,
    handler: tokio::task::JoinHandle<()>,
    fsm_worker: maidan_server::fsm_hook_worker::FsmHookWorker,
    automation_worker: maidan_server::automation_worker::AutomationDeliveryWorker,
    indexer: maidan_search::IndexerHandle,
    client: reqwest::Client,
    handler_state: HandlerState,
    _dir: tempfile::TempDir,
}

impl Harness {
    async fn shutdown(self) {
        self.fsm_worker.shutdown().await;
        self.automation_worker.shutdown().await;
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
        Arc::new(std::sync::atomic::AtomicI64::new(0)),
        None,
    );
    state.webhooks = WebhookRuntime::new(test_key());
    state.slash = SlashRuntime::new(test_key());
    state.fsm_hooks = FsmHookRuntime::new(test_key());
    let indexer = Indexer::new(bus.clone(), Arc::new(LoggingHandler::default()))
        .spawn_with_heartbeat(state.indexer_last_event_unix_ms.clone());
    let fsm_worker = maidan_server::fsm_hook_worker::FsmHookWorker::spawn(state.clone());
    let automation_worker =
        maidan_server::automation_worker::AutomationDeliveryWorker::spawn(state.clone());
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let handler_state = HandlerState {
        secret: Arc::new(Mutex::new(String::new())),
        received: Arc::new(AtomicBool::new(false)),
        last_from: Arc::new(Mutex::new(None)),
        last_to: Arc::new(Mutex::new(None)),
    };
    let handler_app = Router::new()
        .route("/fsm", post(fsm_http_handler))
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
        fsm_worker,
        automation_worker,
        indexer,
        client: reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap(),
        handler_state,
        _dir: dir,
    }
}

async fn wait_for_received(state: &HandlerState, timeout: Duration) -> bool {
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
async fn fsm_hook_invokes_http_handler_on_thread_close() {
    let h = spawn().await;

    let ws: serde_json::Value = h
        .client
        .post(format!("{}/workspaces", h.base))
        .json(&json!({ "name": "fsm" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let wid = ws["id"].as_str().unwrap();

    let actor: serde_json::Value = h
        .client
        .post(format!("{}/workspaces/{wid}/members", h.base))
        .json(&json!({ "handle": "actor", "kind": "agent" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let actor_id = actor["id"].as_str().unwrap();

    let hook_url = format!("http://{}/fsm", h.handler_addr);
    let mint: serde_json::Value = h
        .client
        .post(format!("{}/workspaces/{wid}/fsm-hooks", h.base))
        .json(&json!({
            "label": "on-close",
            "from_state": "in_review",
            "to_state": "closed",
            "handler_kind": "http",
            "handler_target": hook_url
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

    let _: serde_json::Value = h
        .client
        .post(format!("{}/threads/{tid}", h.base))
        .json(&json!({ "actor_id": actor_id, "action": "start_review" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    h.handler_state.received.store(false, Ordering::SeqCst);
    let _: serde_json::Value = h
        .client
        .post(format!("{}/threads/{tid}", h.base))
        .json(&json!({ "actor_id": actor_id, "action": "close" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    assert!(
        wait_for_received(&h.handler_state, Duration::from_secs(5)).await,
        "fsm hook did not receive transition"
    );
    assert_eq!(
        h.handler_state.last_from.lock().await.as_deref(),
        Some("in_review")
    );
    assert_eq!(
        h.handler_state.last_to.lock().await.as_deref(),
        Some("closed")
    );

    h.shutdown().await;
}

#[tokio::test]
async fn fsm_hook_does_not_fire_when_states_do_not_match() {
    let h = spawn().await;

    let ws: serde_json::Value = h
        .client
        .post(format!("{}/workspaces", h.base))
        .json(&json!({ "name": "nomatch" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let wid = ws["id"].as_str().unwrap();

    let actor: serde_json::Value = h
        .client
        .post(format!("{}/workspaces/{wid}/members", h.base))
        .json(&json!({ "handle": "a", "kind": "agent" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let actor_id = actor["id"].as_str().unwrap();

    let hook_url = format!("http://{}/fsm", h.handler_addr);
    let _mint: serde_json::Value = h
        .client
        .post(format!("{}/workspaces/{wid}/fsm-hooks", h.base))
        .json(&json!({
            "from_state": "open",
            "to_state": "archived",
            "handler_kind": "http",
            "handler_target": hook_url
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

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
        .post(format!("{}/threads/{tid}", h.base))
        .json(&json!({ "actor_id": actor_id, "action": "start_review" }))
        .send()
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !h.handler_state.received.load(Ordering::SeqCst),
        "hook should not match open -> in_review"
    );

    h.shutdown().await;
}

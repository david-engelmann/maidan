//! Cluster 98: dedicated mention webhook config and `mention_recorded` delivery.

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
    state.received.store(true, Ordering::SeqCst);
    StatusCode::OK
}

struct Harness {
    base: String,
    receiver_addr: SocketAddr,
    server: tokio::task::JoinHandle<()>,
    receiver: tokio::task::JoinHandle<()>,
    webhook_worker: WebhookWorker,
    client: reqwest::Client,
    receiver_state: ReceiverState,
    _dir: tempfile::TempDir,
}

impl Harness {
    async fn shutdown(self) {
        self.webhook_worker.shutdown().await;
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
    let webhook_worker = WebhookWorker::spawn(state.clone());
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let receiver_state = ReceiverState {
        secret: Arc::new(Mutex::new(String::new())),
        received: Arc::new(AtomicBool::new(false)),
        last_body: Arc::new(Mutex::new(None)),
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
        client,
        receiver_state,
        _dir: dir,
    }
}

async fn bootstrap_member(h: &Harness, wid: &str, handle: &str) -> String {
    let member: serde_json::Value = h
        .client
        .post(format!("{}/workspaces/{wid}/members", h.base))
        .json(&json!({ "handle": handle, "kind": "agent" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    member["id"].as_str().unwrap().to_string()
}

async fn wait_for_event_kind(state: &ReceiverState, kind: &str, timeout: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    while tokio::time::Instant::now() < deadline {
        if let Some(body) = state.last_body.lock().await.clone() {
            if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&body) {
                if parsed["kind"] == kind {
                    return true;
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    false
}

#[tokio::test]
async fn mention_webhook_config_roundtrip_and_delivers_mention_recorded() {
    let h = spawn().await;

    let ws: serde_json::Value = h
        .client
        .post(format!("{}/workspaces", h.base))
        .json(&json!({ "name": "mention-hook" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let wid = ws["id"].as_str().unwrap();
    let author_id = bootstrap_member(&h, wid, "author").await;
    let mentioned_id = bootstrap_member(&h, wid, "mentioned").await;

    let hook_url = format!("http://{}/hook", h.receiver_addr);
    let mint: serde_json::Value = h
        .client
        .post(format!("{}/workspaces/{wid}/webhooks", h.base))
        .json(&json!({
            "url": hook_url,
            "label": "mention-only",
            "event_kinds": ["vote_cast"]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let webhook_id = mint["webhook"]["id"].as_str().unwrap();
    *h.receiver_state.secret.lock().await = mint["secret"].as_str().unwrap().to_string();

    let empty: serde_json::Value = h
        .client
        .get(format!("{}/workspaces/{wid}/mention-webhook", h.base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(empty["webhook_id"].is_null());

    let set: serde_json::Value = h
        .client
        .put(format!("{}/workspaces/{wid}/mention-webhook", h.base))
        .json(&json!({ "webhook_id": webhook_id }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(set["webhook_id"].as_str().unwrap(), webhook_id);

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

    let msg: serde_json::Value = h
        .client
        .post(format!("{}/threads/{tid}/messages", h.base))
        .json(&json!({ "author_id": author_id, "body": "hi @mentioned" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let mid = msg["id"].as_str().unwrap();

    let mention = h
        .client
        .post(format!("{}/messages/{mid}/mentions", h.base))
        .json(&json!({ "member_id": mentioned_id }))
        .send()
        .await
        .unwrap();
    assert_eq!(mention.status(), StatusCode::NO_CONTENT);

    assert!(
        wait_for_event_kind(
            &h.receiver_state,
            "mention_recorded",
            Duration::from_secs(10)
        )
        .await,
        "mention webhook receiver did not get mention_recorded POST"
    );
    let body = h.receiver_state.last_body.lock().await.clone().unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(parsed["kind"], "mention_recorded");

    h.shutdown().await;
}

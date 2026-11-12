//! Dedicated mention webhook config and `mention_recorded` delivery.

// Mock receivers are plain axum servers, not the API (see clippy.toml).
#![allow(clippy::disallowed_methods)]

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
use maidan_server::{
    router,
    test_support::{self, Tenant},
    webhook_worker::WebhookWorker,
    AppState, WebhookRuntime,
};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::MemberKind;
use serde_json::json;
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
    store: Arc<dyn Store>,
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
    // SAFETY: test setup. Maidan reads this variable only through std,
    // whose environment lock serializes the read with this write.
    unsafe { std::env::set_var("MAIDAN_ALLOW_PRIVATE_EGRESS", "1") };
    let pool = maidan_search::sqlite_pool_options()
        .max_connections(4)
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
    let bus = Arc::new(InMemoryBus::new());
    let mut state = AppState::new(
        store.clone(),
        artifacts,
        bus.clone(),
        search,
        Arc::new(maidan_search::HashV1Provider),
        false,
        false,
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

    Harness {
        base: format!("http://{addr}"),
        receiver_addr,
        server,
        receiver,
        webhook_worker,
        store,
        receiver_state,
        _dir: dir,
    }
}

async fn wait_for_event_kind(state: &ReceiverState, kind: &str, timeout: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    while tokio::time::Instant::now() < deadline {
        if let Some(body) = state.last_body.lock().await.clone()
            && let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&body)
            && parsed["kind"] == kind
        {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    false
}

#[tokio::test]
async fn mention_webhook_config_roundtrip_and_delivers_mention_recorded() {
    let h = spawn().await;

    let tenant = test_support::tenant(h.store.as_ref(), "mention-hook").await;
    let wid = tenant.workspace_id.0;
    let author = tenant
        .member(h.store.as_ref(), "author", MemberKind::Agent)
        .await;
    let mentioned = tenant
        .member(h.store.as_ref(), "mentioned", MemberKind::Agent)
        .await;
    let mentioned_id = mentioned.member_id.0;
    let client = tenant.client();

    let hook_url = format!("http://{}/hook", h.receiver_addr);
    let mint: serde_json::Value = client
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

    let empty: serde_json::Value = client
        .get(format!("{}/workspaces/{wid}/mention-webhook", h.base))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(empty["webhook_id"].is_null());

    let set: serde_json::Value = client
        .put(format!("{}/workspaces/{wid}/mention-webhook", h.base))
        .json(&json!({ "webhook_id": webhook_id }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(set["webhook_id"].as_str().unwrap(), webhook_id);

    let ch: serde_json::Value = client
        .post(format!("{}/workspaces/{wid}/channels", h.base))
        .json(&json!({ "name": "general" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let cid = ch["id"].as_str().unwrap();

    let th: serde_json::Value = client
        .post(format!("{}/channels/{cid}/threads", h.base))
        .json(&json!({ "title": "t" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let tid = th["id"].as_str().unwrap();

    let msg: serde_json::Value = author
        .client()
        .post(format!("{}/threads/{tid}/messages", h.base))
        .json(&json!({ "body": "hi @mentioned" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let mid = msg["id"].as_str().unwrap();

    let mention = client
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

async fn create_workspace(h: &Harness, name: &str) -> Tenant {
    test_support::tenant(h.store.as_ref(), name).await
}

async fn put_mention_webhook(
    h: &Harness,
    tenant: &Tenant,
    webhook_id: &str,
) -> (StatusCode, String) {
    let wid = tenant.workspace_id.0;
    let res = tenant
        .client()
        .put(format!("{}/workspaces/{wid}/mention-webhook", h.base))
        .json(&json!({ "webhook_id": webhook_id }))
        .send()
        .await
        .unwrap();
    let status = res.status();
    (status, res.text().await.unwrap())
}

#[tokio::test]
async fn another_workspaces_subscription_id_gets_the_same_answer_as_a_missing_one() {
    let h = spawn().await;
    let ada = create_workspace(&h, "ada").await;
    let bob = create_workspace(&h, "bob").await;
    let mint: serde_json::Value = bob
        .client()
        .post(format!(
            "{}/workspaces/{}/webhooks",
            h.base, bob.workspace_id.0
        ))
        .json(&json!({
            "url": format!("http://{}/hook", h.receiver_addr),
            "label": "bob-hook",
            "event_kinds": ["vote_cast"]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let bobs_id = mint["webhook"]["id"].as_str().unwrap().to_string();

    let foreign = put_mention_webhook(&h, &ada, &bobs_id).await;
    let missing = put_mention_webhook(&h, &ada, &uuid::Uuid::new_v4().to_string()).await;
    assert_eq!(foreign.0, StatusCode::BAD_REQUEST);
    assert_eq!(
        foreign, missing,
        "a foreign id must be indistinguishable from a missing one"
    );

    // Bob's own workspace can still use it; Ada's setting stayed unset.
    let own = put_mention_webhook(&h, &bob, &bobs_id).await;
    assert_eq!(own.0, StatusCode::OK);
    let ada_cfg: serde_json::Value = ada
        .client()
        .get(format!(
            "{}/workspaces/{}/mention-webhook",
            h.base, ada.workspace_id.0
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(ada_cfg["webhook_id"].is_null());
    h.shutdown().await;
}

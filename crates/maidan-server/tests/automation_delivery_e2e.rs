//! Automation delivery DLQ and replay (Cluster 68.0).

use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use axum::{body::Bytes, extract::State, http::StatusCode, routing::post, Router};
use maidan_bus::InMemoryBus;
use maidan_search::{Indexer, LoggingHandler};
use maidan_server::{
    automation_worker::AutomationDeliveryWorker, fsm_hook_worker::FsmHookWorker, router, AppState,
    FsmHookRuntime, SlashRuntime, WebhookRuntime,
};
use maidan_store::{run_sqlite_migrations, AutomationDeliveryFilter, SqliteStore, Store};
use maidan_types::NewWorkspace;
use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;
use tokio::sync::Mutex;

fn test_key() -> Option<Arc<[u8; 32]>> {
    Some(Arc::new([0x68; 32]))
}

#[derive(Clone)]
struct HandlerState {
    secret: Arc<Mutex<String>>,
    reject: Arc<AtomicBool>,
    received: Arc<AtomicBool>,
}

async fn fsm_http_handler(
    State(state): State<HandlerState>,
    headers: axum::http::HeaderMap,
    body: Bytes,
) -> StatusCode {
    if state.reject.load(Ordering::SeqCst) {
        return StatusCode::SERVICE_UNAVAILABLE;
    }
    let body_str = String::from_utf8_lossy(&body).into_owned();
    let signature = headers
        .get("X-Maidan-Signature")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !maidan_server::webhooks::verify_signature(&state.secret.lock().await, &body_str, signature)
    {
        return StatusCode::UNAUTHORIZED;
    }
    state.received.store(true, Ordering::SeqCst);
    StatusCode::OK
}

#[tokio::test]
async fn fsm_http_delivery_quarantines_then_replay_succeeds() {
    std::env::set_var("MAIDAN_AUTOMATION_MAX_ATTEMPTS", "1");
    std::env::set_var("MAIDAN_AUTOMATION_POLL_INTERVAL_MS", "10");

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
    let store_for_assert = store.clone();
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
    let fsm_worker = FsmHookWorker::spawn(state.clone());
    let automation_worker = AutomationDeliveryWorker::spawn(state.clone());
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let base = format!("http://{addr}");

    let handler_state = HandlerState {
        secret: Arc::new(Mutex::new(String::new())),
        reject: Arc::new(AtomicBool::new(true)),
        received: Arc::new(AtomicBool::new(false)),
    };
    let handler_app = Router::new()
        .route("/fsm", post(fsm_http_handler))
        .with_state(handler_state.clone());
    let handler_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let handler_addr = handler_listener.local_addr().unwrap();
    let handler =
        tokio::spawn(async move { axum::serve(handler_listener, handler_app).await.unwrap() });

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();

    let workspace = store_for_assert
        .create_workspace(NewWorkspace {
            name: "auto-dlq".into(),
        })
        .await
        .unwrap();
    let workspace_id = workspace.id;
    let wid = workspace_id.0.to_string();

    let actor: serde_json::Value = client
        .post(format!("{base}/workspaces/{wid}/members"))
        .json(&json!({ "handle": "op", "kind": "human" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let actor_id = actor["id"].as_str().unwrap();

    let hook_url = format!("http://{handler_addr}/fsm");
    let mint: serde_json::Value = client
        .post(format!("{base}/workspaces/{wid}/fsm-hooks"))
        .json(&json!({
            "from_state": "open",
            "to_state": "in_review",
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
    *handler_state.secret.lock().await = mint["secret"].as_str().unwrap().to_string();

    let ch: serde_json::Value = client
        .post(format!("{base}/workspaces/{wid}/channels"))
        .json(&json!({ "name": "c" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let cid = ch["id"].as_str().unwrap();
    let th: serde_json::Value = client
        .post(format!("{base}/channels/{cid}/threads"))
        .json(&json!({ "title": "t" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let tid = th["id"].as_str().unwrap();

    let _: serde_json::Value = client
        .post(format!("{base}/threads/{tid}"))
        .json(&json!({ "actor_id": actor_id, "action": "start_review" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    let mut delivery_id = None;
    while tokio::time::Instant::now() < deadline {
        let rows = store_for_assert
            .list_automation_deliveries(workspace_id, AutomationDeliveryFilter::DeadLetter, 50)
            .await
            .unwrap();
        if let Some(row) = rows.first() {
            delivery_id = Some(row.id);
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let delivery_id = delivery_id.expect("expected dead-letter automation delivery");

    handler_state.reject.store(false, Ordering::SeqCst);
    let replayed = store_for_assert
        .replay_automation_delivery(delivery_id, workspace_id)
        .await
        .unwrap();
    assert!(replayed.quarantined_at.is_none());

    let replay_http = client
        .post(format!(
            "{base}/workspaces/{wid}/automation/deliveries/{delivery_id}/replay"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(replay_http.status(), reqwest::StatusCode::NOT_FOUND);

    let received_deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < received_deadline {
        if handler_state.received.load(Ordering::SeqCst) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(handler_state.received.load(Ordering::SeqCst));

    fsm_worker.shutdown().await;
    automation_worker.shutdown().await;
    indexer.shutdown().await;
    server.abort();
    handler.abort();
}

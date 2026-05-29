//! Direct messages: open conversation, post, WS filter by dm_conversation_id.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use futures::{SinkExt, StreamExt};
use maidan_artifacts::LocalFsStore;
use maidan_server::{router, subscribe_resume, AppState, FederationRuntime};
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::EventFilter;
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

async fn spawn() -> (SocketAddr, reqwest::Client, tokio::task::JoinHandle<()>) {
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
        true,
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
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    (addr, client, server)
}

#[tokio::test]
async fn dm_open_post_and_list_messages_round_trip() {
    let (addr, client, server) = spawn().await;
    let base = format!("http://{addr}");

    let ws: Value = client
        .post(format!("{base}/workspaces"))
        .json(&json!({"name": "dm-ws"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let workspace_id = ws["id"].as_str().unwrap();
    let alice: Value = client
        .post(format!("{base}/workspaces/{workspace_id}/members"))
        .json(&json!({"handle": "alice", "kind": "human"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let alice_id = alice["id"].as_str().unwrap();
    let bob: Value = client
        .post(format!("{base}/workspaces/{workspace_id}/members"))
        .json(&json!({"handle": "bob", "kind": "agent"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let bob_id = bob["id"].as_str().unwrap();

    let dm: Value = client
        .post(format!("{base}/workspaces/{workspace_id}/dm"))
        .json(&json!({
            "member_id": alice_id,
            "other_member_id": bob_id
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let dm_id = dm["id"].as_str().unwrap();

    let msg: Value = client
        .post(format!("{base}/dm/{dm_id}/messages"))
        .json(&json!({
            "author_id": alice_id,
            "body": "hello bob"
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(msg["body"].as_str(), Some("hello bob"));

    let listed: Vec<Value> = client
        .get(format!("{base}/dm/{dm_id}/messages"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        listed
            .iter()
            .any(|m| m["body"].as_str() == Some("hello bob")),
        "DM message not listed"
    );

    server.abort();
}

#[tokio::test]
async fn ws_subscribe_with_dm_conversation_id_receives_message_posted() {
    let (addr, client, server) = spawn().await;
    let base = format!("http://{addr}");

    let ws: Value = client
        .post(format!("{base}/workspaces"))
        .json(&json!({"name": "dm-ws-ws"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let workspace_id = ws["id"].as_str().unwrap();
    let a: Value = client
        .post(format!("{base}/workspaces/{workspace_id}/members"))
        .json(&json!({"handle": "a", "kind": "human"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let a_id = a["id"].as_str().unwrap();
    let b: Value = client
        .post(format!("{base}/workspaces/{workspace_id}/members"))
        .json(&json!({"handle": "b", "kind": "human"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let b_id = b["id"].as_str().unwrap();

    let dm: Value = client
        .post(format!("{base}/workspaces/{workspace_id}/dm"))
        .json(&json!({ "member_id": a_id, "other_member_id": b_id }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let dm_id = dm["id"].as_str().unwrap();

    let (notify_tx, mut notify_rx) = tokio::sync::mpsc::channel::<String>(4);
    let ws_url = format!("ws://{addr}/ws/subscribe");
    let filter = EventFilter {
        workspace_id: Some(maidan_types::WorkspaceId(
            uuid::Uuid::parse_str(workspace_id).unwrap(),
        )),
        dm_conversation_id: Some(maidan_types::DmConversationId(
            uuid::Uuid::parse_str(dm_id).unwrap(),
        )),
        ..Default::default()
    };
    let subscribe_frame = json!({
        "filter": filter,
        "after_id": 0
    });
    let ws_task = tokio::spawn(async move {
        let (mut socket, _) = tokio_tungstenite::connect_async(&ws_url)
            .await
            .expect("ws connect");
        use tokio_tungstenite::tungstenite::Message as WsMessage;
        socket
            .send(WsMessage::Text(subscribe_frame.to_string()))
            .await
            .expect("subscribe send");
        loop {
            let msg = socket.next().await;
            let Some(Ok(WsMessage::Text(text))) = msg else {
                break;
            };
            if text.contains("message_posted") {
                let _ = notify_tx.send(text).await;
                break;
            }
        }
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let resp = client
        .post(format!("{base}/dm/{dm_id}/messages"))
        .json(&json!({ "author_id": a_id, "body": "via dm filter" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);

    let payload = tokio::time::timeout(Duration::from_secs(5), notify_rx.recv())
        .await
        .expect("timed out waiting for ws dm event")
        .expect("ws task ended");
    assert!(payload.contains("message_posted"));
    assert!(payload.contains("via dm filter"));

    ws_task.abort();
    server.abort();
}

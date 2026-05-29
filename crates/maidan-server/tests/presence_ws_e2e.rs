//! WebSocket presence and typing fan-out.

use std::{sync::Arc, time::Duration};

use futures::{SinkExt, StreamExt};
use maidan_artifacts::LocalFsStore;
use maidan_bus::InMemoryBus;
use maidan_server::{router, subscribe_resume, AppState, FederationRuntime};
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message},
};

async fn spawn() -> (
    std::net::SocketAddr,
    reqwest::Client,
    tokio::task::JoinHandle<()>,
) {
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
    let bus = Arc::new(InMemoryBus::with_capacity(64));
    let mut state = AppState::new(
        store,
        artifacts,
        bus,
        search,
        Arc::new(maidan_search::HashV1Provider),
        true,
        false,
        FederationRuntime::new(true, None),
        Arc::new(std::sync::atomic::AtomicI64::new(0)),
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

fn is_control_frame(v: &serde_json::Value) -> bool {
    matches!(
        v.get("type").and_then(|t| t.as_str()),
        Some("subscribe_ack")
            | Some("replay_hint")
            | Some("replay_truncated")
            | Some("presence_snapshot")
    )
}

async fn next_ephemeral(
    ws: &mut (impl StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin),
) -> serde_json::Value {
    loop {
        let msg = ws.next().await.expect("ws stream").expect("ws ok");
        if let Message::Text(payload) = msg {
            let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
            if is_control_frame(&v) {
                continue;
            }
            return v;
        }
    }
}

#[tokio::test]
async fn presence_and_typing_fan_out_between_subscribers() {
    let (addr, client, server) = spawn().await;
    let base = format!("http://{addr}");
    let ws_url = format!("ws://{addr}/ws/subscribe");

    let ws_resp: serde_json::Value = client
        .post(format!("{base}/workspaces"))
        .json(&json!({"name": "presence-ws"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let workspace_id = ws_resp["id"].as_str().unwrap();

    let alice: serde_json::Value = client
        .post(format!("{base}/workspaces/{workspace_id}/members"))
        .json(&json!({"handle": "alice", "kind": "human"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let bob: serde_json::Value = client
        .post(format!("{base}/workspaces/{workspace_id}/members"))
        .json(&json!({"handle": "bob", "kind": "human"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let alice_id = alice["id"].as_str().unwrap();
    let bob_id = bob["id"].as_str().unwrap();

    let ch: serde_json::Value = client
        .post(format!("{base}/workspaces/{workspace_id}/channels"))
        .json(&json!({"name": "general"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let channel_id = ch["id"].as_str().unwrap();

    let th: serde_json::Value = client
        .post(format!("{base}/channels/{channel_id}/threads"))
        .json(&json!({"title": "t1"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let thread_id = th["id"].as_str().unwrap();

    let filter = json!({
        "filter": { "workspace_id": workspace_id },
        "member_id": alice_id
    });
    let (mut ws_alice, _) = connect_async(ws_url.clone().into_client_request().unwrap())
        .await
        .unwrap();
    ws_alice
        .send(Message::Text(filter.to_string()))
        .await
        .unwrap();

    let filter_bob = json!({
        "filter": { "workspace_id": workspace_id },
        "member_id": bob_id
    });
    let (mut ws_bob, _) = connect_async(ws_url.into_client_request().unwrap())
        .await
        .unwrap();
    ws_bob
        .send(Message::Text(filter_bob.to_string()))
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(50)).await;

    let presence = next_ephemeral(&mut ws_alice).await;
    assert_eq!(presence["type"], "presence");
    assert_eq!(presence["member_id"], bob_id);
    assert_eq!(presence["status"], "online");

    ws_bob
        .send(Message::Text(
            json!({"type": "typing", "thread_id": thread_id, "active": true}).to_string(),
        ))
        .await
        .unwrap();

    let typing = next_ephemeral(&mut ws_alice).await;
    assert_eq!(typing["type"], "typing");
    assert_eq!(typing["member_id"], bob_id);
    assert_eq!(typing["thread_id"], thread_id);
    assert_eq!(typing["active"], true);

    ws_bob
        .send(Message::Text(
            json!({"type": "presence", "status": "away"}).to_string(),
        ))
        .await
        .unwrap();

    let away = next_ephemeral(&mut ws_alice).await;
    assert_eq!(away["type"], "presence");
    assert_eq!(away["member_id"], bob_id);
    assert_eq!(away["status"], "away");

    ws_alice.close(None).await.ok();
    ws_bob.close(None).await.ok();
    server.abort();
}

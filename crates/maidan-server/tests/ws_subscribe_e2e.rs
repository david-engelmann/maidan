//! End-to-end coverage for the `/ws/subscribe` endpoint.
//!
//! Connects a tokio-tungstenite client, sends a Subscribe frame,
//! drives mutations through the HTTP API on a side channel, and
//! asserts the client receives the expected events in order.

use std::{sync::Arc, time::Duration};

use futures::{SinkExt, StreamExt};
use maidan_artifacts::LocalFsStore;
use maidan_bus::InMemoryBus;
use maidan_server::{router, AppState};
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::EventKind;
use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message},
};

async fn spawn_server() -> (
    std::net::SocketAddr,
    reqwest::Client,
    tokio::task::JoinHandle<()>,
    tempfile::TempDir,
) {
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
    let bus = Arc::new(InMemoryBus::with_capacity(256));
    let app = router(AppState::for_tests(store, artifacts, bus, search));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    (addr, client, server, dir)
}

#[tokio::test]
async fn subscribe_receives_matching_events_in_order() {
    let (addr, client, server, _dir) = spawn_server().await;
    let base = format!("http://{addr}");
    let ws_url = format!("ws://{addr}/ws/subscribe");

    let req = ws_url.into_client_request().unwrap();
    let (mut ws, _resp) = connect_async(req).await.expect("ws connect");

    // subscribe to everything
    ws.send(Message::Text(json!({"filter": {}}).to_string()))
        .await
        .unwrap();

    // give the bus subscription a beat to attach before producing events
    tokio::time::sleep(Duration::from_millis(100)).await;

    // produce three events via HTTP
    let ws_resp: serde_json::Value = client
        .post(format!("{base}/workspaces"))
        .json(&json!({"name": "ws-test"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let workspace_id = ws_resp["id"].as_str().unwrap().to_string();
    let _: serde_json::Value = client
        .post(format!("{base}/workspaces/{workspace_id}/members"))
        .json(&json!({"handle": "alice", "kind": "human"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let _: serde_json::Value = client
        .post(format!("{base}/workspaces/{workspace_id}/channels"))
        .json(&json!({"name": "general"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    // collect three text frames
    let mut kinds = Vec::new();
    let collect = async {
        while kinds.len() < 3 {
            match ws.next().await {
                Some(Ok(Message::Text(payload))) => {
                    let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
                    kinds.push(v["kind"].as_str().unwrap().to_string());
                }
                Some(Ok(Message::Ping(_))) | Some(Ok(Message::Pong(_))) => {}
                other => panic!("unexpected ws frame: {other:?}"),
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(5), collect)
        .await
        .expect("timeout waiting for events");

    assert_eq!(
        kinds,
        vec!["workspace_created", "member_joined", "channel_created"]
    );

    ws.close(None).await.ok();
    server.abort();
}

#[tokio::test]
async fn subscribe_filters_by_kind() {
    let (addr, client, server, _dir) = spawn_server().await;
    let base = format!("http://{addr}");
    let ws_url = format!("ws://{addr}/ws/subscribe");

    let req = ws_url.into_client_request().unwrap();
    let (mut ws, _resp) = connect_async(req).await.expect("ws connect");

    // subscribe only to MessagePosted events
    ws.send(Message::Text(
        json!({"filter": {"kinds": [EventKind::MessagePosted]}}).to_string(),
    ))
    .await
    .unwrap();

    tokio::time::sleep(Duration::from_millis(100)).await;

    // setup workspace/member/channel/thread
    let ws_resp: serde_json::Value = client
        .post(format!("{base}/workspaces"))
        .json(&json!({"name": "filter-ws"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let workspace_id = ws_resp["id"].as_str().unwrap().to_string();
    let alice: serde_json::Value = client
        .post(format!("{base}/workspaces/{workspace_id}/members"))
        .json(&json!({"handle": "alice", "kind": "human"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let alice_id = alice["id"].as_str().unwrap().to_string();
    let ch: serde_json::Value = client
        .post(format!("{base}/workspaces/{workspace_id}/channels"))
        .json(&json!({"name": "general"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let channel_id = ch["id"].as_str().unwrap().to_string();
    let th: serde_json::Value = client
        .post(format!("{base}/channels/{channel_id}/threads"))
        .json(&json!({"title": "t"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let thread_id = th["id"].as_str().unwrap().to_string();
    // post two messages — the only events the subscriber should see
    for body in ["one", "two"] {
        let _: serde_json::Value = client
            .post(format!("{base}/threads/{thread_id}/messages"))
            .json(&json!({"author_id": alice_id, "body": body}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
    }

    let mut bodies = Vec::new();
    let collect = async {
        while bodies.len() < 2 {
            match ws.next().await {
                Some(Ok(Message::Text(payload))) => {
                    let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
                    assert_eq!(v["kind"], "message_posted");
                    bodies.push(v["message"]["body"].as_str().unwrap().to_string());
                }
                Some(Ok(Message::Ping(_))) | Some(Ok(Message::Pong(_))) => {}
                other => panic!("unexpected ws frame: {other:?}"),
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(5), collect)
        .await
        .expect("timeout waiting for message_posted events");
    assert_eq!(bodies, vec!["one", "two"]);

    ws.close(None).await.ok();
    server.abort();
}

#[tokio::test]
async fn subscribe_emits_replay_hint_when_bus_subscriber_lags() {
    use maidan_bus::EventBus;

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
    let bus = Arc::new(InMemoryBus::with_capacity(2));
    let app = router(AppState::new(
        store.clone(),
        artifacts,
        bus.clone(),
        search,
        true,
        true,
        Arc::new(std::sync::atomic::AtomicI64::new(0)),
        None,
    ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let req = format!("ws://{addr}/ws/subscribe")
        .into_client_request()
        .unwrap();
    let (mut ws, _resp) = connect_async(req).await.expect("ws connect");
    ws.send(Message::Text(json!({"filter": {}}).to_string()))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(50)).await;

    let ws_id = uuid::Uuid::new_v4();
    for i in 0..12 {
        bus.publish(maidan_types::BusEnvelope::synthetic(
            maidan_types::Event::WorkspaceCreated {
                occurred_at: chrono::Utc::now(),
                workspace: maidan_types::Workspace {
                    id: maidan_types::WorkspaceId(ws_id),
                    name: format!("flood-{i}"),
                    created_at: chrono::Utc::now(),
                    updated_at: chrono::Utc::now(),
                    tombstoned_at: None,
                },
            },
        ))
        .await
        .unwrap();
    }

    let mut saw_hint = false;
    let wait = async {
        while let Some(Ok(msg)) = ws.next().await {
            if let Message::Text(payload) = msg {
                let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
                if v.get("type").and_then(|t| t.as_str()) == Some("replay_hint") {
                    saw_hint = true;
                    assert!(v["skipped"].as_u64().unwrap() > 0);
                    assert!(v["after_id"].is_number());
                    break;
                }
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(2), wait)
        .await
        .expect("timeout waiting for replay_hint");
    assert!(saw_hint);

    server.abort();
}

#[tokio::test]
async fn subscribe_resumes_after_id_from_event_log() {
    let (addr, client, server, _dir) = spawn_server().await;
    let base = format!("http://{addr}");
    let ws_url = format!("ws://{addr}/ws/subscribe");

    let req = ws_url.clone().into_client_request().unwrap();
    let (mut ws, _resp) = connect_async(req).await.expect("ws connect");
    ws.send(Message::Text(json!({"filter": {}}).to_string()))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    let ws_resp: serde_json::Value = client
        .post(format!("{base}/workspaces"))
        .json(&json!({"name": "resume-ws"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let workspace_id = ws_resp["id"].as_str().unwrap();
    let _: serde_json::Value = client
        .post(format!("{base}/workspaces/{workspace_id}/members"))
        .json(&json!({"handle": "alice", "kind": "human"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let _: serde_json::Value = client
        .post(format!("{base}/workspaces/{workspace_id}/channels"))
        .json(&json!({"name": "general"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    let mut first_log_id = None;
    let read_first = async {
        while first_log_id.is_none() {
            if let Some(Ok(Message::Text(payload))) = ws.next().await {
                let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
                if v.get("type").and_then(|t| t.as_str()) != Some("replay_hint") {
                    first_log_id = Some(v["log_id"].as_i64().unwrap());
                    break;
                }
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(3), read_first)
        .await
        .expect("timeout");
    let after_id = first_log_id.unwrap();
    ws.close(None).await.ok();

    let req = ws_url.into_client_request().unwrap();
    let (mut ws, _resp) = connect_async(req).await.expect("ws reconnect");
    ws.send(Message::Text(
        json!({
            "filter": {"workspace_id": workspace_id},
            "after_id": after_id
        })
        .to_string(),
    ))
    .await
    .unwrap();

    let mut kinds = Vec::new();
    let collect = async {
        while kinds.len() < 2 {
            if let Some(Ok(Message::Text(payload))) = ws.next().await {
                let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
                if v.get("type").and_then(|t| t.as_str()) == Some("replay_hint") {
                    continue;
                }
                kinds.push(v["kind"].as_str().unwrap().to_string());
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(3), collect)
        .await
        .expect("timeout on resume");
    assert_eq!(kinds, vec!["member_joined", "channel_created"]);

    ws.close(None).await.ok();
    server.abort();
}

#[tokio::test]
async fn subscribe_with_invalid_filter_closes_with_1008() {
    let (addr, _client, server, _dir) = spawn_server().await;
    let ws_url = format!("ws://{addr}/ws/subscribe");

    let req = ws_url.into_client_request().unwrap();
    let (mut ws, _resp) = connect_async(req).await.expect("ws connect");

    // send something that isn't a SubscribeFrame
    ws.send(Message::Text("not a subscribe frame".to_string()))
        .await
        .unwrap();

    let mut got_close = false;
    let wait_close = async {
        while let Some(msg) = ws.next().await {
            if let Ok(Message::Close(Some(frame))) = msg {
                got_close = true;
                assert_eq!(
                    frame.code,
                    tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::Policy
                );
                break;
            }
        }
    };
    let _ = tokio::time::timeout(Duration::from_secs(2), wait_close).await;
    assert!(got_close, "expected close frame");

    server.abort();
}

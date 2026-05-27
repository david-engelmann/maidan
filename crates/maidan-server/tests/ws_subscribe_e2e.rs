//! End-to-end coverage for the `/ws/subscribe` endpoint.
//!
//! Connects a tokio-tungstenite client, sends a Subscribe frame,
//! drives mutations through the HTTP API on a side channel, and
//! asserts the client receives the expected events in order.

use std::{sync::Arc, time::Duration};

use futures::{SinkExt, StreamExt};
use maidan_artifacts::LocalFsStore;
use maidan_bus::InMemoryBus;
use maidan_server::{router, subscribe_resume, AppState, FederationRuntime};
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

fn is_control_frame(v: &serde_json::Value) -> bool {
    matches!(
        v.get("type").and_then(|t| t.as_str()),
        Some("subscribe_ack") | Some("replay_hint") | Some("replay_truncated")
    )
}

fn with_subscribe_resume_secret(mut state: AppState) -> AppState {
    state.subscribe_resume_secret = Some(Arc::from(subscribe_resume::TEST_SUBSCRIBE_RESUME_SECRET));
    state
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
                    if is_control_frame(&v) {
                        continue;
                    }
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
                    if is_control_frame(&v) {
                        continue;
                    }
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

    maidan_server::metrics::init();

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
    let app = router(with_subscribe_resume_secret(AppState::new(
        store.clone(),
        artifacts,
        bus.clone(),
        search,
        Arc::new(maidan_search::HashV1Provider),
        true,
        false,
        FederationRuntime::new(true, None),
        Arc::new(std::sync::atomic::AtomicI64::new(0)),
        None,
    )));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();

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

    let metrics = client
        .get(format!("http://{addr}/metrics"))
        .send()
        .await
        .expect("metrics")
        .text()
        .await
        .expect("metrics body");
    assert!(
        metrics.contains("maidan_bus_lag_total"),
        "expected bus lag counter in metrics"
    );
    assert!(
        metrics.contains("maidan_subscribe_replay_total") && metrics.contains("replay_hint"),
        "expected replay_hint outcome in metrics"
    );

    server.abort();
}

#[tokio::test]
async fn subscribe_auto_replays_from_event_log_when_bus_lags_with_workspace_filter() {
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
    let app = router(with_subscribe_resume_secret(AppState::new(
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
    )));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let base = format!("http://{addr}");

    let ws_resp: serde_json::Value = client
        .post(format!("{base}/workspaces"))
        .json(&json!({"name": "lag-replay"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let workspace_id = ws_resp["id"].as_str().unwrap().to_string();

    let ws_url = format!("ws://{addr}/ws/subscribe");
    let req = ws_url.into_client_request().unwrap();
    let (mut ws, _resp) = connect_async(req).await.expect("ws connect");
    ws.send(Message::Text(
        json!({"filter": {"workspace_id": workspace_id}}).to_string(),
    ))
    .await
    .unwrap();
    let wait_ack = async {
        loop {
            if let Some(Ok(Message::Text(payload))) = ws.next().await {
                let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
                if v.get("type").and_then(|t| t.as_str()) == Some("subscribe_ack") {
                    break;
                }
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(2), wait_ack)
        .await
        .expect("timeout waiting for subscribe_ack");

    // Drain WS in the background so a full send queue cannot block bus lag replay.
    let (frame_tx, mut frame_rx) = tokio::sync::mpsc::unbounded_channel();
    let ws_reader = tokio::spawn(async move {
        while let Some(Ok(Message::Text(payload))) = ws.next().await {
            let _ = frame_tx.send(payload);
        }
    });

    let flood_client = client.clone();
    let flood_base = base.clone();
    let flood_wid = workspace_id.clone();
    let flood_task = tokio::spawn(async move {
        for i in 0..16 {
            let _: serde_json::Value = flood_client
                .post(format!("{flood_base}/workspaces/{flood_wid}/channels"))
                .json(&json!({"name": format!("ch-{i}")}))
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
        }
    });

    flood_task.await.unwrap();

    let mut saw_channel_created = false;
    let mut saw_replay_hint = false;
    let wait = async {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while !saw_channel_created && tokio::time::Instant::now() < deadline {
            let payload =
                match tokio::time::timeout(Duration::from_millis(500), frame_rx.recv()).await {
                    Ok(Some(p)) => p,
                    Ok(None) | Err(_) => continue,
                };
            let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
            if v.get("type").and_then(|t| t.as_str()) == Some("replay_hint") {
                saw_replay_hint = true;
            } else if is_control_frame(&v) {
                continue;
            } else if v.get("kind").and_then(|k| k.as_str()) == Some("channel_created") {
                saw_channel_created = true;
                assert!(v["log_id"].as_i64().unwrap() > 0);
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(12), wait)
        .await
        .expect("timeout waiting for auto-replayed channel_created");
    ws_reader.abort();
    assert!(saw_channel_created);
    assert!(
        !saw_replay_hint,
        "workspace filter should auto-replay instead of hint"
    );

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
                if !is_control_frame(&v) {
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
                if is_control_frame(&v) {
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
async fn subscribe_reconnects_with_resume_token_only() {
    let (addr, client, server, _dir) = spawn_server().await;
    let base = format!("http://{addr}");
    let ws_url = format!("ws://{addr}/ws/subscribe");

    let ws_resp: serde_json::Value = client
        .post(format!("{base}/workspaces"))
        .json(&json!({"name": "resume-token"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let workspace_id = ws_resp["id"].as_str().unwrap();

    let req = ws_url.clone().into_client_request().unwrap();
    let (mut ws, _resp) = connect_async(req).await.expect("ws connect");
    ws.send(Message::Text(
        json!({"filter": {"workspace_id": workspace_id}}).to_string(),
    ))
    .await
    .unwrap();

    let mut resume_token = None;
    let read_ack = async {
        while resume_token.is_none() {
            if let Some(Ok(Message::Text(payload))) = ws.next().await {
                let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
                if v.get("type").and_then(|t| t.as_str()) == Some("subscribe_ack") {
                    resume_token = Some(v["resume_token"].as_str().unwrap().to_string());
                    assert_eq!(v["after_id"].as_i64(), Some(0));
                }
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(3), read_ack)
        .await
        .expect("timeout waiting for subscribe_ack");
    ws.close(None).await.ok();

    let _: serde_json::Value = client
        .post(format!("{base}/workspaces/{workspace_id}/members"))
        .json(&json!({"handle": "bob", "kind": "human"}))
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

    let req = ws_url.into_client_request().unwrap();
    let (mut ws, _resp) = connect_async(req).await.expect("ws reconnect");
    ws.send(Message::Text(
        json!({"resume_token": resume_token.unwrap()}).to_string(),
    ))
    .await
    .unwrap();

    let mut kinds = Vec::new();
    let collect = async {
        while kinds.len() < 3 {
            if let Some(Ok(Message::Text(payload))) = ws.next().await {
                let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
                if is_control_frame(&v) {
                    continue;
                }
                kinds.push(v["kind"].as_str().unwrap().to_string());
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(3), collect)
        .await
        .expect("timeout on resume_token reconnect");
    assert_eq!(
        kinds,
        vec!["workspace_created", "member_joined", "channel_created"]
    );

    ws.close(None).await.ok();
    server.abort();
}

#[tokio::test]
async fn subscribe_with_invalid_resume_token_closes_with_1008() {
    let (addr, _client, server, _dir) = spawn_server().await;
    let ws_url = format!("ws://{addr}/ws/subscribe");

    let req = ws_url.into_client_request().unwrap();
    let (mut ws, _resp) = connect_async(req).await.expect("ws connect");
    ws.send(Message::Text(
        json!({"resume_token": "not.a.valid.token"}).to_string(),
    ))
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
    assert!(got_close, "expected close frame for invalid resume_token");

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

#[tokio::test]
async fn subscribe_receives_many_sequential_events() {
    let (addr, client, server, _dir) = spawn_server().await;
    let base = format!("http://{addr}");
    let ws_url = format!("ws://{addr}/ws/subscribe");

    let req = ws_url.into_client_request().unwrap();
    let (mut ws, _resp) = connect_async(req).await.expect("ws connect");
    ws.send(Message::Text(json!({"filter": {}}).to_string()))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;

    let ws_resp: serde_json::Value = client
        .post(format!("{base}/workspaces"))
        .json(&json!({"name": "soak"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let workspace_id = ws_resp["id"].as_str().unwrap();

    const N: usize = 100;
    for i in 0..N {
        let _: serde_json::Value = client
            .post(format!("{base}/workspaces/{workspace_id}/members"))
            .json(&json!({"handle": format!("u{i}"), "kind": "agent"}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
    }

    let mut count = 0usize;
    let collect = async {
        while count < N {
            match ws.next().await {
                Some(Ok(Message::Text(payload))) => {
                    let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
                    if is_control_frame(&v) {
                        continue;
                    }
                    if v["kind"].as_str() == Some(EventKind::MemberJoined.as_str()) {
                        count += 1;
                    }
                }
                Some(Ok(Message::Ping(_))) | Some(Ok(Message::Pong(_))) => {}
                other => panic!("unexpected ws frame: {other:?}"),
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(30), collect)
        .await
        .expect("timeout waiting for 100 member events");

    server.abort();
}

#[tokio::test]
async fn subscribe_emits_replay_truncated_when_event_log_exceeds_replay_limit() {
    use maidan_server::event_stream::REPLAY_LIMIT;

    let (addr, client, server, _dir) = spawn_server().await;
    let base = format!("http://{addr}");
    let ws_url = format!("ws://{addr}/ws/subscribe");

    let ws_resp: serde_json::Value = client
        .post(format!("{base}/workspaces"))
        .json(&json!({"name": "trunc-ws"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let workspace_id = ws_resp["id"].as_str().unwrap();

    for i in 0..REPLAY_LIMIT {
        let _: serde_json::Value = client
            .post(format!("{base}/workspaces/{workspace_id}/members"))
            .json(&json!({"handle": format!("m{i}"), "kind": "agent"}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
    }

    let req = ws_url.into_client_request().unwrap();
    let (mut ws, _resp) = connect_async(req).await.expect("ws connect");
    ws.send(Message::Text(
        json!({
            "filter": {"workspace_id": workspace_id},
            "after_id": 1
        })
        .to_string(),
    ))
    .await
    .unwrap();

    let mut truncated = None;
    let wait = async {
        while truncated.is_none() {
            if let Some(Ok(Message::Text(payload))) = ws.next().await {
                let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
                if v.get("type").and_then(|t| t.as_str()) == Some("replay_truncated") {
                    truncated = Some(v);
                    break;
                }
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(60), wait)
        .await
        .expect("timeout waiting for replay_truncated");
    let frame = truncated.unwrap();
    assert_eq!(frame["limit"].as_i64(), Some(REPLAY_LIMIT));
    assert_eq!(
        frame["workspace_id"].as_str(),
        Some(workspace_id),
        "workspace_id should be present"
    );
    assert!(
        frame["after_id"].as_i64().unwrap() > 1,
        "truncated watermark should advance past after_id=1"
    );

    ws.close(None).await.ok();
    server.abort();
}

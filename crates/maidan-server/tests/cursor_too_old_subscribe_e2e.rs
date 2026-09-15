//! Cluster 388.2: subscribe / HTTP backfill fail loud on a pruned-gap cursor
//! (409 `must_refetch`, never a silent clamp) and honour projector shapes.

use std::{sync::Arc, time::Duration};

use futures::{SinkExt, StreamExt};
use maidan_artifacts::LocalFsStore;
use maidan_bus::InMemoryBus;
use maidan_server::{router, AppState};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{Event, MemberKind, NewChannel, NewMember, NewWorkspace};
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message},
};

async fn spawn() -> (
    std::net::SocketAddr,
    reqwest::Client,
    tokio::task::JoinHandle<()>,
    tempfile::TempDir,
    Arc<dyn Store>,
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
    let app = router(AppState::for_tests(store.clone(), artifacts, bus, search));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    (addr, client, server, dir, store)
}

async fn seed(store: &dyn Store) -> (maidan_types::WorkspaceId, Vec<i64>) {
    let ws = store
        .create_workspace(NewWorkspace {
            name: "cursor-ws".into(),
        })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "u".into(),
            display_name: None,
            kind: MemberKind::Human,
        })
        .await
        .unwrap();
    let ch = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "c".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let e1 = store
        .append_event(&Event::MemberJoined {
            occurred_at: chrono::Utc::now(),
            workspace_id: ws.id,
            member: member.clone(),
        })
        .await
        .unwrap();
    let e2 = store
        .append_event(&Event::ChannelCreated {
            occurred_at: chrono::Utc::now(),
            workspace_id: ws.id,
            channel: ch,
        })
        .await
        .unwrap();
    let e3 = store
        .append_event(&Event::MemberJoined {
            occurred_at: chrono::Utc::now(),
            workspace_id: ws.id,
            member,
        })
        .await
        .unwrap();
    (ws.id, vec![e1.id, e2.id, e3.id])
}

#[tokio::test]
async fn list_events_returns_409_must_refetch_when_cursor_is_in_pruned_gap() {
    let (addr, client, server, _dir, store) = spawn().await;
    let (ws, ids) = seed(store.as_ref()).await;
    let cutoff = chrono::Utc::now() + chrono::Duration::hours(1);
    let deleted = store.prune_events(cutoff, ids[1], 10).await.unwrap();
    assert!(deleted >= 2);

    let resp = client
        .get(format!(
            "http://{addr}/workspaces/{}/events?after_id={}&limit=10",
            ws.0, ids[0]
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(
        body["type"].as_str(),
        Some("https://maidan.dev/problems/cursor-too-old")
    );
    assert_eq!(body["must_refetch"], json!(true));

    // Fresh subscriber and adjacent resume stay 200.
    let fresh = client
        .get(format!(
            "http://{addr}/workspaces/{}/events?after_id=0&limit=10",
            ws.0
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(fresh.status(), StatusCode::OK);
    let adjacent = client
        .get(format!(
            "http://{addr}/workspaces/{}/events?after_id={}&limit=10",
            ws.0, ids[1]
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(adjacent.status(), StatusCode::OK);

    server.abort();
}

#[tokio::test]
async fn list_events_filters_by_projector_shape_types() {
    let (addr, client, server, _dir, store) = spawn().await;
    let (ws, _ids) = seed(store.as_ref()).await;

    let all: Vec<Value> = client
        .get(format!(
            "http://{addr}/workspaces/{}/events?after_id=0&limit=50",
            ws.0
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(all.len() >= 3);

    let only: Vec<Value> = client
        .get(format!(
            "http://{addr}/workspaces/{}/events?after_id=0&limit=50&types=channel_created",
            ws.0
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(!only.is_empty());
    assert!(only.iter().all(|e| e["kind"] == "channel_created"));
    assert!(only.iter().all(|e| {
        e["$type"] == "maidan.event.channel_created/1" && e["kind"] == "channel_created"
    }));

    let bad = client
        .get(format!(
            "http://{addr}/workspaces/{}/events?after_id=0&types=not_a_kind",
            ws.0
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), StatusCode::BAD_REQUEST);

    server.abort();
}

#[tokio::test]
async fn list_events_stamps_type_on_stored_event_for_known_kinds() {
    let (addr, client, server, _dir, store) = spawn().await;
    let (ws, _ids) = seed(store.as_ref()).await;

    let events: Vec<Value> = client
        .get(format!(
            "http://{addr}/workspaces/{}/events?after_id=0&limit=50",
            ws.0
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(!events.is_empty());
    for event in &events {
        let kind = event["kind"].as_str().expect("kind");
        assert_eq!(
            event["$type"].as_str(),
            Some(format!("maidan.event.{kind}/1").as_str()),
            "HTTP StoredEvent must carry lexicon $type for {kind}"
        );
        assert!(
            event["payload"].get("$type").is_none(),
            "stored payload is not rewritten; $type is the row envelope"
        );
    }

    server.abort();
}

#[tokio::test]
async fn durable_consumer_cursor_that_points_into_a_pruned_gap_is_409() {
    let (addr, client, server, _dir, store) = spawn().await;
    let (ws, ids) = seed(store.as_ref()).await;
    store
        .advance_delivery_cursor("proj-1", ws, ids[0])
        .await
        .unwrap();
    let cutoff = chrono::Utc::now() + chrono::Duration::hours(1);
    store.prune_events(cutoff, ids[1], 10).await.unwrap();

    let resp = client
        .get(format!(
            "http://{addr}/workspaces/{}/events?after_id=0&consumer_id=proj-1",
            ws.0
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["must_refetch"], json!(true));

    server.abort();
}

#[tokio::test]
async fn ws_subscribe_sends_cursor_too_old_frame_then_closes() {
    let (addr, _client, server, _dir, store) = spawn().await;
    let (ws_id, ids) = seed(store.as_ref()).await;
    let cutoff = chrono::Utc::now() + chrono::Duration::hours(1);
    store.prune_events(cutoff, ids[1], 10).await.unwrap();

    let ws_url = format!("ws://{addr}/ws/subscribe");
    let req = ws_url.into_client_request().unwrap();
    let (mut socket, _resp) = connect_async(req).await.expect("ws connect");
    socket
        .send(Message::Text(
            json!({
                "filter": { "workspace_id": ws_id.0 },
                "after_id": ids[0]
            })
            .to_string(),
        ))
        .await
        .unwrap();

    let mut saw_too_old = false;
    while let Some(Ok(msg)) = socket.next().await {
        match msg {
            Message::Text(payload) => {
                let v: Value = serde_json::from_str(&payload).unwrap();
                if v["type"] == "cursor_too_old" {
                    assert_eq!(v["must_refetch"], json!(true));
                    assert_eq!(v["after_id"], ids[0]);
                    saw_too_old = true;
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
    assert!(
        saw_too_old,
        "expected cursor_too_old control frame before close"
    );

    server.abort();
}

#[tokio::test]
async fn mcp_stream_returns_409_must_refetch_when_cursor_is_in_pruned_gap() {
    let (addr, client, server, _dir, store) = spawn().await;
    let (ws, ids) = seed(store.as_ref()).await;
    let cutoff = chrono::Utc::now() + chrono::Duration::hours(1);
    store.prune_events(cutoff, ids[1], 10).await.unwrap();

    let resp = client
        .get(format!(
            "http://{addr}/mcp/stream?workspace_id={}&after_id={}",
            ws.0, ids[0]
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CONFLICT);
    let body: Value = resp.json().await.unwrap();
    assert_eq!(
        body["type"].as_str(),
        Some("https://maidan.dev/problems/cursor-too-old")
    );
    assert_eq!(body["must_refetch"], json!(true));

    let fresh = client
        .get(format!(
            "http://{addr}/mcp/stream?workspace_id={}&after_id=0",
            ws.0
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(fresh.status(), StatusCode::OK);

    server.abort();
}

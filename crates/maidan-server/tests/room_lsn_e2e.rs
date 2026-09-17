//! `Maidan-Room-LSN` is the event-log high-water (decimal), always stamped on
//! REST / WS / MCP / A2A. Distinct from `Maidan-Consistency-Token` (WAL LSN,
//! replica-gated). SQLite in-process.

use std::{sync::Arc, time::Duration};

use futures::{SinkExt, StreamExt};
use maidan_artifacts::LocalFsStore;
use maidan_bus::InMemoryBus;
use maidan_server::{consistency::CONSISTENCY_TOKEN_HEADER, router, AppState};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{Event, MemberKind, NewChannel, NewMember, NewWorkspace, ROOM_LSN_HEADER};
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

async fn seed(store: &dyn Store) -> (maidan_types::WorkspaceId, i64) {
    let ws = store
        .create_workspace(NewWorkspace {
            name: "room-lsn-ws".into(),
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
    store
        .append_event(&Event::MemberJoined {
            occurred_at: chrono::Utc::now(),
            workspace_id: ws.id,
            member: member.clone(),
        })
        .await
        .unwrap();
    let last = store
        .append_event(&Event::ChannelCreated {
            occurred_at: chrono::Utc::now(),
            workspace_id: ws.id,
            channel: ch,
        })
        .await
        .unwrap();
    (ws.id, last.id)
}

fn header_lsn(resp: &reqwest::Response) -> i64 {
    resp.headers()
        .get(ROOM_LSN_HEADER)
        .expect("Maidan-Room-LSN")
        .to_str()
        .unwrap()
        .parse()
        .expect("decimal room lsn")
}

#[tokio::test]
async fn rest_stamps_room_lsn_and_never_a_consistency_token_on_sqlite() {
    let (addr, client, server, _dir, store) = spawn().await;
    let (ws, last) = seed(store.as_ref()).await;
    assert_eq!(store.max_event_id().await.unwrap(), last);

    let live = client
        .get(format!("http://{addr}/health/live"))
        .send()
        .await
        .unwrap();
    assert_eq!(live.status(), StatusCode::OK);
    assert!(
        live.headers().get(ROOM_LSN_HEADER).is_none(),
        "liveness must not wait on the event log"
    );

    let resp = client
        .get(format!(
            "http://{addr}/workspaces/{}/events?after_id=0&limit=10",
            ws.0
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(
        resp.headers().get(CONSISTENCY_TOKEN_HEADER).is_none(),
        "Room-LSN is not the replica causality token"
    );
    assert_eq!(header_lsn(&resp), last);

    let a2a = client
        .post(format!("http://{addr}/a2a/v1/rpc"))
        .json(&json!({"jsonrpc":"2.0","id":1,"method":"nope"}))
        .send()
        .await
        .unwrap();
    assert_eq!(header_lsn(&a2a), last);

    server.abort();
}

#[tokio::test]
async fn ws_and_mcp_stamp_header_ack_and_type_on_frames() {
    let (addr, client, server, _dir, store) = spawn().await;
    let (ws_id, last) = seed(store.as_ref()).await;

    let req = format!("ws://{addr}/ws/subscribe")
        .into_client_request()
        .unwrap();
    let (mut ws, upgrade) = connect_async(req).await.expect("ws connect");
    let upgrade_lsn: i64 = upgrade
        .headers()
        .get(ROOM_LSN_HEADER)
        .expect("upgrade Maidan-Room-LSN")
        .to_str()
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(upgrade_lsn, last);

    ws.send(Message::Text(
        json!({
            "filter": {"workspace_id": ws_id.0},
            "after_id": 1
        })
        .to_string(),
    ))
    .await
    .unwrap();

    let mut replayed: Option<Value> = None;
    let ack = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            match ws.next().await {
                Some(Ok(Message::Text(t))) => {
                    let v: Value = serde_json::from_str(&t).unwrap();
                    if v.get("type").and_then(|t| t.as_str()) == Some("subscribe_ack") {
                        return v;
                    }
                    if v.get("kind").is_some() {
                        replayed = Some(v);
                    }
                }
                other => panic!("unexpected before ack: {other:?}"),
            }
        }
    })
    .await
    .expect("subscribe_ack");
    assert_eq!(ack["room_lsn"], last);
    let frame = replayed.expect("replayed a domain frame before ack");
    let type_id = frame["$type"].as_str().expect("$type");
    assert!(
        type_id.starts_with("maidan.event.") && type_id.ends_with("/1"),
        "{type_id}"
    );
    assert!(frame.get("kind").and_then(Value::as_str).is_some());

    let mcp = client
        .get(format!(
            "http://{addr}/mcp/stream?workspace_id={}&after_id=0",
            ws_id.0
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(mcp.status(), StatusCode::OK);
    assert_eq!(header_lsn(&mcp), last);

    server.abort();
}

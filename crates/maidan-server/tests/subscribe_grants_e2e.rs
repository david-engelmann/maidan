//! WS subscribe channel grants (Cluster 81.0).

use std::{sync::Arc, time::Duration};

use futures::{SinkExt, StreamExt};
use maidan_artifacts::LocalFsStore;
use maidan_bus::InMemoryBus;
use maidan_server::{router, subscribe_grants, AppState};
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
    let bus = Arc::new(InMemoryBus::with_capacity(64));
    let app = router(AppState::for_tests(store, artifacts, bus, search));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    (addr, client, server)
}

#[tokio::test]
async fn subscribe_denies_private_channel_without_grant() {
    let (addr, client, server) = spawn().await;
    let base = format!("http://{addr}");
    let ws_url = format!("ws://{addr}/ws/subscribe");

    let ws: serde_json::Value = client
        .post(format!("{base}/workspaces"))
        .json(&json!({"name": "grants-ws"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let workspace_id = ws["id"].as_str().unwrap();

    let req = ws_url.clone().into_client_request().unwrap();
    let (mut socket, _resp) = connect_async(req).await.unwrap();
    socket
        .send(Message::Text(
            json!({
                "filter": {
                    "workspace_id": workspace_id,
                    "channel_grants": []
                }
            })
            .to_string(),
        ))
        .await
        .unwrap();

    let mut past_ack = false;
    let ack_deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while tokio::time::Instant::now() < ack_deadline {
        let Some(Ok(Message::Text(payload))) = socket.next().await else {
            break;
        };
        let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
        if v.get("type").and_then(|t| t.as_str()) == Some("subscribe_ack") {
            past_ack = true;
            break;
        }
    }
    assert!(past_ack);

    let public_ch: serde_json::Value = client
        .post(format!("{base}/workspaces/{workspace_id}/channels"))
        .json(&json!({"name": "public", "private": false}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let public_id = public_ch["id"].as_str().unwrap();

    let private_ch: serde_json::Value = client
        .post(format!("{base}/workspaces/{workspace_id}/channels"))
        .json(&json!({"name": "secret", "private": true}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let private_id = private_ch["id"].as_str().unwrap();

    let mut saw_public = false;
    let mut saw_private = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline && (!saw_public || !saw_private) {
        let Ok(Some(Ok(Message::Text(payload)))) =
            tokio::time::timeout(Duration::from_millis(500), socket.next()).await
        else {
            continue;
        };
        let v: serde_json::Value = serde_json::from_str(&payload).unwrap();
        if v.get("type").is_some() {
            continue;
        }
        if v.get("kind").and_then(|k| k.as_str()) == Some("channel_created") {
            let ch = v["channel"]["id"].as_str().unwrap();
            if ch == private_id {
                saw_private = true;
            }
            if ch == public_id {
                saw_public = true;
            }
        }
    }
    assert!(saw_public, "expected public channel_created");
    assert!(
        !saw_private,
        "must not receive private channel without grant"
    );

    let req2 = ws_url.clone().into_client_request().unwrap();
    let (mut socket2, _) = connect_async(req2).await.unwrap();
    socket2
        .send(Message::Text(
            json!({
                "filter": {
                    "workspace_id": workspace_id,
                    "channel_id": private_id
                }
            })
            .to_string(),
        ))
        .await
        .unwrap();
    let close = tokio::time::timeout(Duration::from_secs(2), socket2.next())
        .await
        .expect("expected close")
        .expect("stream ended")
        .expect("ws frame");
    match close {
        Message::Close(frame) => {
            let code: u16 = frame.map(|f| f.code.into()).unwrap_or(1000);
            assert_eq!(code, 1008);
        }
        other => panic!("expected close frame, got {other:?}"),
    }

    server.abort();
}

#[tokio::test]
async fn subscribe_grants_helper_builds_filter() {
    let ws = maidan_types::WorkspaceId(uuid::Uuid::new_v4());
    let ch = maidan_types::ChannelId(uuid::Uuid::new_v4());
    let filter = subscribe_grants::workspace_filter(ws, &[ch]);
    assert_eq!(filter.workspace_id, Some(ws));
    assert_eq!(filter.channel_grants.as_deref(), Some(&[ch][..]));
}

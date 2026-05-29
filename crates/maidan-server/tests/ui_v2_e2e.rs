//! UI v2 shell: channel list API, layout markers, WS live tail.

use std::{sync::Arc, time::Duration};

use futures::{SinkExt, StreamExt};
use maidan_artifacts::LocalFsStore;
use maidan_bus::InMemoryBus;
use maidan_server::{router, AppState};
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
    let app = router(AppState::for_tests(store, artifacts, bus, search));
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
async fn ui_v2_shell_lists_channels_via_session_api() {
    let (addr, client, server) = spawn().await;
    let base = format!("http://{addr}");

    let html = client
        .get(format!("{base}/ui/"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    assert!(html.contains(r#"data-ui-version="3""#));
    assert!(html.contains(r#"id="shell""#));
    assert!(html.contains(r#"id="channel-list""#));
    assert!(html.contains(r#"id="live-feed""#));
    assert!(html.contains("Connect WS"));

    let ws: serde_json::Value = client
        .post(format!("{base}/workspaces"))
        .json(&json!({"name": "ui-v2"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let workspace_id = ws["id"].as_str().unwrap();

    client
        .post(format!("{base}/workspaces/{workspace_id}/channels"))
        .json(&json!({"name": "general", "private": false}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();

    let channels: Vec<serde_json::Value> = client
        .get(format!("{base}/ui/api/workspaces/{workspace_id}/channels"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(channels.len(), 1);
    assert_eq!(channels[0]["name"], "general");

    server.abort();
}

#[tokio::test]
async fn ui_v2_ws_live_tail_receives_workspace_events() {
    let (addr, client, server) = spawn().await;
    let base = format!("http://{addr}");
    let ws_url = format!("ws://{addr}/ws/subscribe");

    let ws: serde_json::Value = client
        .post(format!("{base}/workspaces"))
        .json(&json!({"name": "live-ws"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let _workspace_id = ws["id"].as_str().unwrap();

    let (mut socket, _) = connect_async(ws_url.into_client_request().unwrap())
        .await
        .unwrap();
    socket
        .send(Message::Text(json!({"filter": {}}).to_string()))
        .await
        .unwrap();

    let got_ack = async {
        loop {
            let msg = socket.next().await.expect("frame").expect("ok");
            if let Message::Text(t) = msg {
                let v: serde_json::Value = serde_json::from_str(&t).unwrap();
                if v.get("type").and_then(|x| x.as_str()) == Some("subscribe_ack") {
                    return;
                }
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(3), got_ack)
        .await
        .expect("subscribe_ack timeout");

    client
        .post(format!("{base}/workspaces"))
        .json(&json!({"name": "second-ws"}))
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap();

    let got_event = async {
        loop {
            let msg = socket.next().await.expect("frame").expect("ok");
            if let Message::Text(t) = msg {
                let v: serde_json::Value = serde_json::from_str(&t).unwrap();
                if v.get("kind").and_then(|k| k.as_str()) == Some("workspace_created") {
                    return;
                }
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(5), got_event)
        .await
        .expect("live event timeout");

    server.abort();
}

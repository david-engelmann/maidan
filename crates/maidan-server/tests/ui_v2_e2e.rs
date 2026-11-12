//! UI v2 shell: channel list API, layout markers, WS live tail.

use std::{sync::Arc, time::Duration};

use futures::{SinkExt, StreamExt};
use maidan_artifacts::LocalFsStore;
use maidan_bus::InMemoryBus;
use maidan_server::{router, test_support, AppState};
use maidan_store::{prelude::*, run_sqlite_migrations};
use serde_json::json;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message},
};

async fn spawn() -> (
    std::net::SocketAddr,
    test_support::Tenant,
    tokio::task::JoinHandle<()>,
) {
    let pool = maidan_search::sqlite_pool_options()
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
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(InMemoryBus::with_capacity(64));
    let tenant = test_support::tenant(store.as_ref(), "ui-v2").await;
    let app = router(AppState::for_tests(store, artifacts, bus, search));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, tenant, server)
}

#[tokio::test]
async fn ui_v2_shell_lists_channels_via_session_api() {
    let (addr, tenant, server) = spawn().await;
    let client = tenant.client();
    let base = format!("http://{addr}");

    let html = client
        .get(format!("{base}/ui/"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    // `/ui/` is the console; its channel list is drawn by the app, so the
    // session API below is what this test holds.
    assert!(html.contains(r#"id="root""#), "{html}");
    assert!(html.contains(r#"src="/ui/app.js""#), "{html}");

    let workspace_id = tenant.workspace_id.0;

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
    let (addr, tenant, server) = spawn().await;
    let client = tenant.client();
    let base = format!("http://{addr}");
    let workspace_id = tenant.workspace_id.0;
    let ws_url = format!("ws://{addr}/ws/subscribe");

    let (mut socket, _) = connect_async(ws_url.into_client_request().unwrap())
        .await
        .unwrap();
    socket
        .send(Message::Text(
            json!({"token": tenant.token, "filter": {}}).to_string(),
        ))
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

    // A subscriber sees only its own workspace's events, so the event it
    // waits for is a change inside the caller's workspace.
    client
        .post(format!("{base}/workspaces/{workspace_id}/channels"))
        .json(&json!({"name": "live", "private": false}))
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
                if v.get("kind").and_then(|k| k.as_str()) == Some("channel_created") {
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

//! `/ws/subscribe` is bounded.
//!
//! Frames a client sends are capped, so one oversized frame cannot bypass the
//! request-body limit every REST route is held to; and the number of live
//! subscriber connections is capped, so long-lived sockets cannot exhaust the
//! process. A slot frees as soon as its socket closes.

use std::{sync::Arc, time::Duration};

use futures::{SinkExt, StreamExt};
use maidan_artifacts::LocalFsStore;
use maidan_bus::InMemoryBus;
use maidan_server::{router, subscribe_resume, AppState};
use maidan_store::{prelude::*, run_sqlite_migrations};
use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message},
};

async fn spawn_server(max_connections: usize) -> (std::net::SocketAddr, tempfile::TempDir) {
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
    let mut state = AppState::for_tests(
        store,
        Arc::new(LocalFsStore::new(dir.path())),
        Arc::new(InMemoryBus::with_capacity(256)),
        search,
    );
    state.subscribe_resume_secret = Some(Arc::from(subscribe_resume::TEST_SUBSCRIBE_RESUME_SECRET));
    state.max_ws_connections = max_connections;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = router(state);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, dir)
}

type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn subscribed(addr: std::net::SocketAddr) -> Socket {
    let url = format!("ws://{addr}/ws/subscribe");
    let (mut ws, _) = connect_async(url.into_client_request().unwrap())
        .await
        .expect("connect");
    ws.send(Message::Text(json!({ "filter": {} }).to_string()))
        .await
        .unwrap();
    loop {
        let frame = tokio::time::timeout(Duration::from_secs(5), ws.next())
            .await
            .expect("ack within 5s");
        if let Some(Ok(Message::Text(text))) = frame {
            if text.contains("subscribe_ack") {
                return ws;
            }
        }
    }
}

#[tokio::test]
async fn an_oversized_client_frame_closes_the_connection() {
    let (addr, _dir) = spawn_server(100).await;
    let mut ws = subscribed(addr).await;
    let oversized = "x".repeat(maidan_server::ws::MAX_CLIENT_WS_MESSAGE_BYTES + 1);
    let _ = ws.send(Message::Text(oversized)).await;
    let closed = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match ws.next().await {
                None | Some(Err(_)) | Some(Ok(Message::Close(_))) => return true,
                Some(Ok(_)) => continue,
            }
        }
    })
    .await
    .expect("the server must end the connection");
    assert!(closed);
}

#[tokio::test]
async fn subscriber_connections_are_capped_and_a_slot_frees_on_close() {
    let (addr, _dir) = spawn_server(1).await;
    let mut first = subscribed(addr).await;

    let url = format!("ws://{addr}/ws/subscribe");
    let refused = connect_async(url.clone().into_client_request().unwrap()).await;
    match refused {
        Err(tokio_tungstenite::tungstenite::Error::Http(resp)) => {
            assert_eq!(resp.status(), 503);
        }
        other => panic!("a connection past the ceiling must get 503, got {other:?}"),
    }

    first.close(None).await.unwrap();
    drop(first);
    // The slot frees when the socket task ends; allow it a moment.
    let mut reopened = None;
    for _ in 0..50 {
        if let Ok((ws, _)) = connect_async(url.clone().into_client_request().unwrap()).await {
            reopened = Some(ws);
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        reopened.is_some(),
        "a closed connection must give back its slot"
    );
}

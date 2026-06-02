//! MCP streamable HTTP: response + notification on `POST /mcp/streamable`.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use futures::StreamExt;
use maidan_artifacts::LocalFsStore;
use maidan_server::{router, subscribe_resume, AppState, FederationRuntime};
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
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
async fn streamable_post_returns_sse_response_and_resource_notification() {
    let (addr, client, server) = spawn().await;
    let base = format!("http://{addr}");

    let ws: Value = client
        .post(format!("{base}/workspaces"))
        .json(&json!({"name": "streamable-ws"}))
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
    let ch: Value = client
        .post(format!("{base}/workspaces/{workspace_id}/channels"))
        .json(&json!({"name": "general"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let channel_id = ch["id"].as_str().unwrap();
    let th: Value = client
        .post(format!("{base}/channels/{channel_id}/threads"))
        .json(&json!({"title": "stream"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let thread_id = th["id"].as_str().unwrap();
    let uri = format!("maidan://threads/{thread_id}");

    let subscribe_body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "resources/subscribe",
        "params": { "uri": uri }
    });
    let sub_resp: Value = client
        .post(format!("{base}/mcp"))
        .json(&subscribe_body)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(sub_resp["error"].is_null());

    let post_body = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "post_message",
            "arguments": {
                "thread_id": thread_id,
                "author_id": alice_id,
                "body": "via streamable"
            }
        }
    });
    let resp2 = client
        .post(format!("{base}/mcp/streamable"))
        .json(&post_body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp2.status(), StatusCode::OK);
    let mut buf = String::new();
    let mut stream = resp2.bytes_stream();
    while let Some(chunk) = stream.next().await {
        buf.push_str(&String::from_utf8_lossy(&chunk.unwrap()));
        if buf.contains("\"id\":2") && buf.contains("notifications/resources/updated") {
            break;
        }
    }
    assert!(buf.contains(&uri));

    server.abort();
}

#[tokio::test]
async fn streamable_response_includes_mcp_session_id_header() {
    let (addr, client, server) = spawn().await;
    let base = format!("http://{addr}");
    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {}
    });
    let resp = client
        .post(format!("{base}/mcp/streamable"))
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let session = resp
        .headers()
        .get("mcp-session-id")
        .expect("mcp-session-id header")
        .to_str()
        .unwrap()
        .to_string();
    assert!(!session.is_empty());

    let resp2 = client
        .post(format!("{base}/mcp/streamable"))
        .header("mcp-session-id", &session)
        .json(&body)
        .send()
        .await
        .unwrap();
    let same = resp2
        .headers()
        .get("mcp-session-id")
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(same, session);

    server.abort();
}

#[tokio::test]
async fn streamable_follow_up_multiplexes_response_on_open_sse_session() {
    let (addr, client, server) = spawn().await;
    let base = format!("http://{addr}");
    let init = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {}
    });
    let resp = client
        .post(format!("{base}/mcp/streamable"))
        .json(&init)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let session = resp
        .headers()
        .get("mcp-session-id")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();

    let list = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list",
        "params": {}
    });

    let mut sse_buf = String::new();
    let mut stream = resp.bytes_stream();
    let list_clone = list.clone();
    let client2 = client.clone();
    let base2 = base.clone();
    let session2 = session.clone();
    let reader = tokio::spawn(async move {
        while let Some(chunk) = stream.next().await {
            sse_buf.push_str(&String::from_utf8_lossy(&chunk.unwrap()));
            if sse_buf.contains("\"id\":2") && sse_buf.contains("tools") {
                break;
            }
        }
        sse_buf
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let resp2 = client2
        .post(format!("{base2}/mcp/streamable"))
        .header("mcp-session-id", &session2)
        .json(&list_clone)
        .send()
        .await
        .unwrap();
    assert_eq!(resp2.status(), StatusCode::ACCEPTED);
    assert!(
        resp2
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_none_or(|ct| !ct.contains("application/json")),
        "follow-up should not return JSON body"
    );

    let buf = tokio::time::timeout(Duration::from_secs(5), reader)
        .await
        .expect("timed out waiting for SSE mux")
        .expect("reader task");
    assert!(buf.contains("\"id\":2"));
    assert!(buf.contains("result"));

    server.abort();
}

#[tokio::test]
async fn streamable_delete_closes_session() {
    let (addr, client, server) = spawn().await;
    let base = format!("http://{addr}");
    let init = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {}
    });
    let resp = client
        .post(format!("{base}/mcp/streamable"))
        .json(&init)
        .send()
        .await
        .unwrap();
    let session = resp
        .headers()
        .get("mcp-session-id")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();

    let del = client
        .delete(format!("{base}/mcp/streamable"))
        .header("mcp-session-id", &session)
        .send()
        .await
        .unwrap();
    assert_eq!(del.status(), StatusCode::NO_CONTENT);

    let resp2 = client
        .post(format!("{base}/mcp/streamable"))
        .header("mcp-session-id", &session)
        .json(&init)
        .send()
        .await
        .unwrap();
    assert_eq!(resp2.status(), StatusCode::OK);
    let new_session = resp2
        .headers()
        .get("mcp-session-id")
        .unwrap()
        .to_str()
        .unwrap();
    assert_ne!(new_session, session.as_str());

    server.abort();
}

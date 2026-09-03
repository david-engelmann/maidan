//! MCP streamable HTTP: response + notification on `POST /mcp/streamable`.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use futures::StreamExt;
use maidan_artifacts::LocalFsStore;
use maidan_server::{router, subscribe_resume, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

async fn spawn() -> (SocketAddr, reqwest::Client, tokio::task::JoinHandle<()>) {
    let (addr, client, server, _mcp) = spawn_with_mcp().await;
    (addr, client, server)
}

async fn spawn_with_mcp() -> (
    SocketAddr,
    reqwest::Client,
    tokio::task::JoinHandle<()>,
    Arc<maidan_mcp::McpServer>,
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
    let mcp = state.mcp.clone();
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    (addr, client, server, mcp)
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
async fn streamable_2026_request_is_stateless_and_mints_no_session() {
    // A 2026-07-28 client omits any session id and even accepts SSE, yet the POST
    // must land cold, return a single JSON-RPC response, and mint NO Mcp-Session-Id
    // (sessions were removed in the 2026-07-28 revision — Protocols.md J3.3-4).
    let (addr, client, server) = spawn().await;
    let base = format!("http://{addr}");
    let body = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {} });
    let resp = client
        .post(format!("{base}/mcp/streamable"))
        .header("MCP-Protocol-Version", "2026-07-28")
        .header("accept", "text/event-stream")
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    assert!(
        resp.headers().get("mcp-session-id").is_none(),
        "a 2026 stateless POST must not mint an Mcp-Session-Id"
    );
    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(
        content_type.contains("application/json"),
        "2026 stateless POST should return inline JSON, got {content_type}"
    );
    let v: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(v["id"], 1);
    assert!(
        v["result"]["tools"].is_array(),
        "tools/list returns a tools array cold, without a session"
    );

    server.abort();
}

#[tokio::test]
async fn streamable_rejects_a_routing_header_that_contradicts_the_body() {
    // SEP-2243: a gateway that routed on Mcp-Method must not be handed a body with
    // a different method — the server rejects the mismatch with 400 (J3.2).
    let (addr, client, server) = spawn().await;
    let base = format!("http://{addr}");
    let body = json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {} });
    let resp = client
        .post(format!("{base}/mcp/streamable"))
        .header("mcp-method", "tools/call")
        .json(&body)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);

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

#[tokio::test]
async fn streamable_get_delivers_server_notification() {
    let (addr, client, server) = spawn().await;
    let base = format!("http://{addr}");

    let ws: Value = client
        .post(format!("{base}/workspaces"))
        .json(&json!({"name": "get-sse-ws"}))
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

    // Subscribe to the resource, then open the server→client GET SSE stream.
    let _: Value = client
        .post(format!("{base}/mcp"))
        .json(&json!({"jsonrpc":"2.0","id":1,"method":"resources/subscribe","params":{"uri":uri}}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let get_resp = client
        .get(format!("{base}/mcp/streamable"))
        .send()
        .await
        .unwrap();
    assert_eq!(get_resp.status(), StatusCode::OK);

    // Trigger an update; it must arrive on the already-open GET stream.
    let _ = client
        .post(format!("{base}/mcp"))
        .json(&json!({
            "jsonrpc":"2.0","id":2,"method":"tools/call",
            "params":{"name":"post_message","arguments":{
                "thread_id": thread_id, "author_id": alice_id, "body": "hi"}}
        }))
        .send()
        .await
        .unwrap();

    let mut buf = String::new();
    let mut stream = get_resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        buf.push_str(&String::from_utf8_lossy(&chunk.unwrap()));
        if buf.contains("notifications/resources/updated") {
            break;
        }
    }
    assert!(
        buf.contains(&uri),
        "GET stream should carry the resource-updated notification"
    );

    server.abort();
}

#[tokio::test]
async fn streamable_post_with_json_accept_returns_single_json() {
    let (addr, client, server) = spawn().await;
    let base = format!("http://{addr}");

    let resp = client
        .post(format!("{base}/mcp/streamable"))
        .header("accept", "application/json")
        .json(&json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let content_type = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert!(
        content_type.contains("application/json"),
        "JSON Accept should yield a JSON body, got {content_type}"
    );
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["id"], 1);
    assert!(body["result"]["protocolVersion"].is_string());

    server.abort();
}

#[tokio::test]
async fn streamable_get_replays_after_last_event_id() {
    let (addr, client, server) = spawn().await;
    let base = format!("http://{addr}");

    // Open a session with `initialize` (SSE). The response is session event id 0.
    let init = client
        .post(format!("{base}/mcp/streamable"))
        .json(&json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}))
        .send()
        .await
        .unwrap();
    assert_eq!(init.status(), StatusCode::OK);
    let session = init
        .headers()
        .get("mcp-session-id")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    // Drop the SSE stream; the session (and its replay log) survives for reconnect.
    drop(init);

    // A follow-up on the open session logs event id 1 (the tools/list response),
    // whether it muxes (202) or answers inline (200) after the leg dropped.
    let follow = client
        .post(format!("{base}/mcp/streamable"))
        .header("mcp-session-id", &session)
        .json(&json!({"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}))
        .send()
        .await
        .unwrap();
    assert!(
        follow.status() == StatusCode::ACCEPTED || follow.status() == StatusCode::OK,
        "follow-up status {}",
        follow.status()
    );

    // Reconnect via GET with Last-Event-ID: 0 → the retained event id 1 is replayed.
    let get = client
        .get(format!("{base}/mcp/streamable"))
        .header("mcp-session-id", &session)
        .header("last-event-id", "0")
        .send()
        .await
        .unwrap();
    assert_eq!(get.status(), StatusCode::OK);
    let mut buf = String::new();
    let mut stream = get.bytes_stream();
    while let Some(chunk) = stream.next().await {
        buf.push_str(&String::from_utf8_lossy(&chunk.unwrap()));
        if buf.contains("\"id\":2") {
            break;
        }
    }
    assert!(
        buf.contains("\"id\":2"),
        "replay should include the tools/list response"
    );
    assert!(
        buf.contains("id: 1"),
        "replayed frame carries its SSE event id"
    );

    server.abort();
}

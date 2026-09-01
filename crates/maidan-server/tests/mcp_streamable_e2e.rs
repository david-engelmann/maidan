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

#[tokio::test]
async fn server_to_client_request_round_trips_over_http() {
    let (addr, client, server, mcp) = spawn_with_mcp().await;
    let base = format!("http://{addr}");

    // Open a session; the client declares the `sampling` capability.
    let init = client
        .post(format!("{base}/mcp/streamable"))
        .json(&json!({
            "jsonrpc":"2.0","id":1,"method":"initialize",
            "params":{"protocolVersion":"2024-11-05","capabilities":{"sampling":{}}}
        }))
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

    // The client opens the canonical server→client GET stream for this session
    // (Cluster 154 — server→client requests are delivered here, not on the POST
    // leg). Subscribing happens as the handler builds the response, so it is
    // established by the time `send()` returns its head.
    let get_resp = client
        .get(format!("{base}/mcp/streamable"))
        .header("mcp-session-id", &session)
        .send()
        .await
        .unwrap();
    assert_eq!(get_resp.status(), StatusCode::OK);

    // The server issues a sampling request to the client…
    let mcp_bg = mcp.clone();
    let session_bg = session.clone();
    let call = tokio::spawn(async move {
        mcp_bg
            .request_client(
                &session_bg,
                "sampling/createMessage",
                json!({"prompt": "hi"}),
            )
            .await
    });

    // …which arrives on the GET stream; read out its JSON-RPC id.
    let mut buf = String::new();
    let mut stream = get_resp.bytes_stream();
    let mut request_id = None;
    while let Some(chunk) = stream.next().await {
        buf.push_str(&String::from_utf8_lossy(&chunk.unwrap()));
        if let Some(line) = buf
            .lines()
            .find(|l| l.starts_with("data:") && l.contains("sampling/createMessage"))
        {
            let data = line.trim_start_matches("data:").trim();
            let req: Value = serde_json::from_str(data).unwrap();
            request_id = req["id"].as_i64();
            break;
        }
    }
    let request_id = request_id.expect("server→client request delivered on the GET stream");

    // The client POSTs its response; the awaiting server call resolves.
    let resp = client
        .post(format!("{base}/mcp/streamable"))
        .header("mcp-session-id", &session)
        .json(&json!({"jsonrpc":"2.0","id":request_id,"result":{"text":"sure"}}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::ACCEPTED);

    let result = call.await.unwrap().unwrap();
    assert_eq!(result, json!({"text":"sure"}));

    server.abort();
}

/// The `summarize_thread` tool is the first organic caller of `request_client`:
/// a `tools/call` gathers the thread and asks the *client* to sample a summary
/// over the canonical GET stream (Cluster 155). This drives the whole Cluster
/// 154 delivery path end-to-end through a real feature.
#[tokio::test]
async fn summarize_thread_tool_samples_via_the_client() {
    let (addr, client, server) = spawn().await;
    let base = format!("http://{addr}");

    // Seed a thread with a couple of messages.
    let ws: Value = client
        .post(format!("{base}/workspaces"))
        .json(&json!({"name": "sum-ws"}))
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
        .json(&json!({"title": "release plan"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let thread_id = th["id"].as_str().unwrap().to_string();
    for body in ["ship on friday", "blocked on the migration"] {
        let _: Value = client
            .post(format!("{base}/threads/{thread_id}/messages"))
            .json(&json!({"author_id": alice_id, "body": body}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
    }

    // Open a streamable session declaring `sampling`, then the GET stream.
    let init = client
        .post(format!("{base}/mcp/streamable"))
        .json(&json!({
            "jsonrpc":"2.0","id":1,"method":"initialize",
            "params":{"protocolVersion":"2024-11-05","capabilities":{"sampling":{}}}
        }))
        .send()
        .await
        .unwrap();
    let session = init
        .headers()
        .get("mcp-session-id")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let get_resp = client
        .get(format!("{base}/mcp/streamable"))
        .header("mcp-session-id", &session)
        .send()
        .await
        .unwrap();
    assert_eq!(get_resp.status(), StatusCode::OK);

    // Call the tool (JSON accept → the result comes back in the POST body). It
    // blocks server-side until the client answers the sampling request, so
    // spawn it and drive the client side concurrently.
    let call_client = client.clone();
    let call_base = base.clone();
    let call_session = session.clone();
    let call_thread = thread_id.clone();
    let call = tokio::spawn(async move {
        call_client
            .post(format!("{call_base}/mcp/streamable"))
            .header("mcp-session-id", &call_session)
            .header("accept", "application/json")
            .json(&json!({
                "jsonrpc":"2.0","id":2,"method":"tools/call",
                "params":{"name":"summarize_thread","arguments":{"thread_id": call_thread}}
            }))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap()
    });

    // The sampling request arrives on the GET stream; read its JSON-RPC id.
    let mut buf = String::new();
    let mut stream = get_resp.bytes_stream();
    let mut request_id = None;
    while let Some(chunk) = stream.next().await {
        buf.push_str(&String::from_utf8_lossy(&chunk.unwrap()));
        if let Some(line) = buf
            .lines()
            .find(|l| l.starts_with("data:") && l.contains("sampling/createMessage"))
        {
            let data = line.trim_start_matches("data:").trim();
            let req: Value = serde_json::from_str(data).unwrap();
            // The transcript the server sent the client includes the messages.
            let text = req["params"]["messages"][0]["content"]["text"]
                .as_str()
                .unwrap();
            assert!(text.contains("ship on friday"));
            request_id = req["id"].as_i64();
            break;
        }
    }
    let request_id = request_id.expect("sampling request delivered on the GET stream");

    // The client returns its sampled summary.
    let resp = client
        .post(format!("{base}/mcp/streamable"))
        .header("mcp-session-id", &session)
        .json(&json!({
            "jsonrpc":"2.0","id":request_id,
            "result":{"role":"assistant","content":{"type":"text","text":"SUMMARY: friday, migration blocker"}}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::ACCEPTED);

    // The tool call resolves, carrying the client's summary.
    let body = call.await.unwrap();
    assert!(
        body.contains("SUMMARY: friday, migration blocker"),
        "tools/call result should carry the client's sampled summary: {body}"
    );

    server.abort();
}

/// The `request_approval` tool (Cluster 174) is the elicitation analogue of
/// `summarize_thread`: `tools/call` asks the *human* on the client to approve an
/// action via a server→client `elicitation/create` over the GET stream, and the
/// human's `accept` resolves the call as `approved: true`.
#[tokio::test]
async fn request_approval_tool_elicits_the_human_via_the_client() {
    let (addr, client, server) = spawn().await;
    let base = format!("http://{addr}");

    // Open a streamable session declaring `elicitation`, then the GET stream.
    let init = client
        .post(format!("{base}/mcp/streamable"))
        .json(&json!({
            "jsonrpc":"2.0","id":1,"method":"initialize",
            "params":{"protocolVersion":"2024-11-05","capabilities":{"elicitation":{}}}
        }))
        .send()
        .await
        .unwrap();
    let session = init
        .headers()
        .get("mcp-session-id")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let get_resp = client
        .get(format!("{base}/mcp/streamable"))
        .header("mcp-session-id", &session)
        .send()
        .await
        .unwrap();
    assert_eq!(get_resp.status(), StatusCode::OK);

    // Call request_approval; it blocks server-side until the human answers.
    let call_client = client.clone();
    let call_base = base.clone();
    let call_session = session.clone();
    let call = tokio::spawn(async move {
        call_client
            .post(format!("{call_base}/mcp/streamable"))
            .header("mcp-session-id", &call_session)
            .header("accept", "application/json")
            .json(&json!({
                "jsonrpc":"2.0","id":2,"method":"tools/call",
                "params":{"name":"request_approval","arguments":{"prompt": "Deploy v9 to prod?"}}
            }))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap()
    });

    // The elicitation/create request arrives on the GET stream.
    let mut buf = String::new();
    let mut stream = get_resp.bytes_stream();
    let mut request_id = None;
    while let Some(chunk) = stream.next().await {
        buf.push_str(&String::from_utf8_lossy(&chunk.unwrap()));
        if let Some(line) = buf
            .lines()
            .find(|l| l.starts_with("data:") && l.contains("elicitation/create"))
        {
            let data = line.trim_start_matches("data:").trim();
            let req: Value = serde_json::from_str(data).unwrap();
            assert_eq!(req["params"]["message"], "Deploy v9 to prod?");
            request_id = req["id"].as_i64();
            break;
        }
    }
    let request_id = request_id.expect("elicitation request delivered on the GET stream");

    // The human accepts.
    let resp = client
        .post(format!("{base}/mcp/streamable"))
        .header("mcp-session-id", &session)
        .json(&json!({
            "jsonrpc":"2.0","id":request_id,
            "result":{"action":"accept","content":{"note":"approved by oncall"}}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::ACCEPTED);

    // The tool call resolves as approved. The result is a text content block
    // whose text is the serialized approval JSON — parse both layers.
    let body = call.await.unwrap();
    let outer: Value = serde_json::from_str(&body).unwrap();
    let text = outer["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("expected a text content block: {body}"));
    let inner: Value = serde_json::from_str(text).unwrap();
    assert_eq!(inner["approved"], json!(true), "body={body}");
    assert_eq!(inner["action"], "accept");
    assert_eq!(inner["content"]["note"], "approved by oncall");

    server.abort();
}

#[tokio::test]
async fn list_roots_tool_queries_the_client() {
    let (addr, client, server) = spawn().await;
    let base = format!("http://{addr}");

    // Open a streamable session declaring `roots`, then the GET stream.
    let init = client
        .post(format!("{base}/mcp/streamable"))
        .json(&json!({
            "jsonrpc":"2.0","id":1,"method":"initialize",
            "params":{"protocolVersion":"2024-11-05","capabilities":{"roots":{}}}
        }))
        .send()
        .await
        .unwrap();
    let session = init
        .headers()
        .get("mcp-session-id")
        .unwrap()
        .to_str()
        .unwrap()
        .to_string();
    let get_resp = client
        .get(format!("{base}/mcp/streamable"))
        .header("mcp-session-id", &session)
        .send()
        .await
        .unwrap();
    assert_eq!(get_resp.status(), StatusCode::OK);

    // Call list_roots; it blocks server-side until the client answers.
    let call_client = client.clone();
    let call_base = base.clone();
    let call_session = session.clone();
    let call = tokio::spawn(async move {
        call_client
            .post(format!("{call_base}/mcp/streamable"))
            .header("mcp-session-id", &call_session)
            .header("accept", "application/json")
            .json(&json!({
                "jsonrpc":"2.0","id":2,"method":"tools/call",
                "params":{"name":"list_roots","arguments":{}}
            }))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap()
    });

    // The roots/list request arrives on the GET stream.
    let mut buf = String::new();
    let mut stream = get_resp.bytes_stream();
    let mut request_id = None;
    while let Some(chunk) = stream.next().await {
        buf.push_str(&String::from_utf8_lossy(&chunk.unwrap()));
        if let Some(line) = buf
            .lines()
            .find(|l| l.starts_with("data:") && l.contains("roots/list"))
        {
            let data = line.trim_start_matches("data:").trim();
            let req: Value = serde_json::from_str(data).unwrap();
            request_id = req["id"].as_i64();
            break;
        }
    }
    let request_id = request_id.expect("roots/list request delivered on the GET stream");

    // The client returns its roots.
    let resp = client
        .post(format!("{base}/mcp/streamable"))
        .header("mcp-session-id", &session)
        .json(&json!({
            "jsonrpc":"2.0","id":request_id,
            "result":{"roots":[{"uri":"file:///work","name":"work"}]}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::ACCEPTED);

    // The tool resolves with the client's roots (a text content block whose text
    // is the serialized roots JSON — parse both layers).
    let body = call.await.unwrap();
    let outer: Value = serde_json::from_str(&body).unwrap();
    let text = outer["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("expected a text content block: {body}"));
    let inner: Value = serde_json::from_str(text).unwrap();
    assert_eq!(inner["roots"][0]["uri"], "file:///work", "body={body}");
    assert_eq!(inner["roots"][0]["name"], "work");

    server.abort();
}

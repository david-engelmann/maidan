//! End-to-end MCP test: HTTP POST /mcp drives a full
//! initialize → tools/list → tools/call → resources/read sequence.

use std::{sync::Arc, time::Duration};

use base64::Engine;
use futures::StreamExt;
use maidan_artifacts::LocalFsStore;
use maidan_bus::InMemoryBus;
use maidan_server::{router, AppState};
use maidan_store::{prelude::*, run_sqlite_migrations};
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

async fn spawn() -> (
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
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
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

async fn rpc(client: &reqwest::Client, base: &str, id: u64, method: &str, params: Value) -> Value {
    rpc_with_member(client, base, id, method, params, None).await
}

async fn rpc_as(
    client: &reqwest::Client,
    base: &str,
    id: u64,
    method: &str,
    params: Value,
    member_id: &str,
) -> Value {
    rpc_with_member(client, base, id, method, params, Some(member_id)).await
}

async fn rpc_with_member(
    client: &reqwest::Client,
    base: &str,
    id: u64,
    method: &str,
    params: Value,
    member_id: Option<&str>,
) -> Value {
    let body = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    });
    let mut request = client.post(format!("{base}/mcp")).json(&body);
    if let Some(member_id) = member_id {
        request = request.header("maidan-test-member-id", member_id);
    }
    request.send().await.unwrap().json().await.unwrap()
}

fn unwrap_tool_text(result: &Value) -> Value {
    let text = result["content"][0]["text"].as_str().unwrap();
    serde_json::from_str(text).unwrap()
}

#[tokio::test]
async fn full_mcp_flow() {
    let (addr, client, server, _dir) = spawn().await;
    let base = format!("http://{addr}");

    // initialize — a version-less client negotiates the current default (2026-07-28).
    let init = rpc(&client, &base, 1, "initialize", json!({})).await;
    assert_eq!(init["jsonrpc"], "2.0");
    assert_eq!(init["id"], 1);
    assert_eq!(init["result"]["protocolVersion"], "2026-07-28");
    assert!(init["result"]["capabilities"]["tools"].is_object());

    // tools/list
    let tools = rpc(&client, &base, 2, "tools/list", json!({})).await;
    let names: Vec<&str> = tools["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"list_channels"));
    assert!(names.contains(&"post_message"));
    assert!(names.contains(&"edit_message"));
    assert!(names.contains(&"add_reference"));

    // need a workspace/member/channel/thread to exercise tools
    let ws_resp: Value = client
        .post(format!("{base}/workspaces"))
        .json(&json!({"name": "mcp-ws"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let workspace_id = ws_resp["id"].as_str().unwrap().to_string();
    let alice: Value = client
        .post(format!("{base}/workspaces/{workspace_id}/members"))
        .json(&json!({"handle": "alice", "kind": "human"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let alice_id = alice["id"].as_str().unwrap().to_string();
    let ch: Value = client
        .post(format!("{base}/workspaces/{workspace_id}/channels"))
        .json(&json!({"name": "general"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let channel_id = ch["id"].as_str().unwrap().to_string();
    let th: Value = client
        .post(format!("{base}/channels/{channel_id}/threads"))
        .json(&json!({"title": "via-mcp"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let thread_id = th["id"].as_str().unwrap().to_string();

    // tools/call: list_channels
    let resp = rpc_as(
        &client,
        &base,
        3,
        "tools/call",
        json!({
            "name": "list_channels",
            "arguments": {"workspace_id": workspace_id}
        }),
        &alice_id,
    )
    .await;
    let channels = unwrap_tool_text(&resp["result"]);
    assert_eq!(channels.as_array().unwrap().len(), 1);

    // tools/call: post_message
    let resp = rpc_as(
        &client,
        &base,
        4,
        "tools/call",
        json!({
            "name": "post_message",
            "arguments": {
                "thread_id": thread_id,
                "body": "hi from mcp"
            }
        }),
        &alice_id,
    )
    .await;
    let posted = unwrap_tool_text(&resp["result"]);
    assert_eq!(posted["body"], "hi from mcp");
    let msg_id = posted["id"].as_str().unwrap().to_string();

    // tools/call: edit_message
    let resp = rpc_as(
        &client,
        &base,
        41,
        "tools/call",
        json!({
            "name": "edit_message",
            "arguments": {
                "message_id": msg_id,
                "body": "edited via mcp"
            }
        }),
        &alice_id,
    )
    .await;
    let edited = unwrap_tool_text(&resp["result"]);
    assert_eq!(edited["body"], "edited via mcp");

    // tools/call: list_messages
    let resp = rpc_as(
        &client,
        &base,
        5,
        "tools/call",
        json!({
            "name": "list_messages",
            "arguments": {"thread_id": thread_id, "limit": 10}
        }),
        &alice_id,
    )
    .await;
    let messages = unwrap_tool_text(&resp["result"]);
    let msgs = messages.as_array().unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0]["body"], "edited via mcp");

    // resources/list names only concrete, readable URIs; the id-addressed
    // resources are templates.
    let resources = rpc(&client, &base, 6, "resources/list", json!({})).await;
    for resource in resources["result"]["resources"].as_array().unwrap() {
        let uri = resource["uri"].as_str().unwrap();
        assert!(
            !uri.contains('{'),
            "resources/list listed a template: {uri}"
        );
    }
    let templates = rpc(&client, &base, 6, "resources/templates/list", json!({})).await;
    let templates: Vec<&str> = templates["result"]["resourceTemplates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["uriTemplate"].as_str().unwrap())
        .collect();
    assert!(templates.contains(&"maidan://workspaces/{id}"));
    assert!(templates.contains(&"maidan://threads/{id}"));

    // resources/read: thread transcript
    let resp = rpc(
        &client,
        &base,
        7,
        "resources/read",
        json!({"uri": format!("maidan://threads/{thread_id}")}),
    )
    .await;
    let contents = &resp["result"]["contents"][0];
    assert_eq!(contents["mimeType"], "application/json");
    let payload: Value = serde_json::from_str(contents["text"].as_str().unwrap()).unwrap();
    assert_eq!(payload["thread"]["id"], thread_id);
    assert_eq!(payload["messages"].as_array().unwrap().len(), 1);

    let prompts = rpc(&client, &base, 8, "prompts/list", json!({})).await;
    let names: Vec<&str> = prompts["result"]["prompts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"thread_workflow"));

    let prompt = rpc(
        &client,
        &base,
        9,
        "prompts/get",
        json!({
            "name": "thread_workflow",
            "arguments": {"thread_id": thread_id}
        }),
    )
    .await;
    let text = prompt["result"]["messages"][0]["content"]["text"]
        .as_str()
        .unwrap();
    assert!(text.contains("open"));

    let artifact_b64 =
        base64::engine::general_purpose::STANDARD.encode(b"artifact body via mcp tool");
    let resp = rpc_as(
        &client,
        &base,
        10,
        "tools/call",
        json!({
            "name": "upload_artifact",
            "arguments": {
                "kind": "transcript",
                "content_base64": artifact_b64
            }
        }),
        &alice_id,
    )
    .await;
    let artifact = unwrap_tool_text(&resp["result"]);
    let sha = artifact["sha256"].as_str().unwrap();
    assert_eq!(artifact["kind"], "transcript");

    let resp = rpc(
        &client,
        &base,
        11,
        "resources/read",
        json!({"uri": format!("maidan://artifacts/{sha}")}),
    )
    .await;
    let contents = &resp["result"]["contents"][0];
    let payload: Value = serde_json::from_str(contents["text"].as_str().unwrap()).unwrap();
    assert_eq!(payload["byte_length"], 26);

    // unknown method
    let resp = rpc(&client, &base, 12, "non/existent", json!({})).await;
    assert!(resp["error"].is_object());
    assert_eq!(resp["error"]["code"], -32601);

    // resources/read with bogus uri scheme
    let resp = rpc(
        &client,
        &base,
        13,
        "resources/read",
        json!({"uri": "http://nope/1"}),
    )
    .await;
    assert!(resp["error"].is_object());
    assert_eq!(resp["error"]["code"], -32602);

    server.abort();
}

#[tokio::test]
async fn http_resource_subscribe_delivers_sse_notification() {
    let (addr, client, server, _dir) = spawn().await;
    let base = format!("http://{addr}");

    let ws_resp: Value = client
        .post(format!("{base}/workspaces"))
        .json(&json!({"name": "mcp-notify-ws"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let workspace_id = ws_resp["id"].as_str().unwrap();
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
        .json(&json!({"title": "notify"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let thread_id = th["id"].as_str().unwrap();
    let uri = format!("maidan://threads/{thread_id}");

    let (notify_tx, mut notify_rx) = tokio::sync::mpsc::channel::<String>(4);
    let sse_client = client.clone();
    let sse_base = base.clone();
    let sse_task = tokio::spawn(async move {
        let resp = sse_client
            .get(format!("{sse_base}/mcp/notifications"))
            .send()
            .await
            .unwrap();
        let mut stream = resp.bytes_stream();
        let mut buf = String::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.unwrap();
            buf.push_str(&String::from_utf8_lossy(&chunk));
            if buf.contains("notifications/resources/updated") {
                let _ = notify_tx.send(buf).await;
                break;
            }
        }
    });

    let subscribe = rpc(
        &client,
        &base,
        1,
        "resources/subscribe",
        json!({ "uri": uri }),
    )
    .await;
    assert!(subscribe["error"].is_null());

    let _ = rpc_as(
        &client,
        &base,
        2,
        "tools/call",
        json!({
            "name": "post_message",
            "arguments": {
                "thread_id": thread_id,
                "body": "notify me"
            }
        }),
        alice_id,
    )
    .await;

    let payload = tokio::time::timeout(Duration::from_secs(5), notify_rx.recv())
        .await
        .expect("timed out waiting for SSE notification")
        .expect("SSE collector exited without notification");
    assert!(payload.contains("notifications/resources/updated"));
    assert!(payload.contains(&uri));

    sse_task.abort();
    server.abort();
}

#[tokio::test]
async fn http_tombstone_emits_resource_updated_sse_notification() {
    let (addr, client, server, _dir) = spawn().await;
    let base = format!("http://{addr}");

    let ws_resp: Value = client
        .post(format!("{base}/workspaces"))
        .json(&json!({"name": "tombstone-notify"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let workspace_id = ws_resp["id"].as_str().unwrap();
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
        .json(&json!({"title": "t"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let thread_id = th["id"].as_str().unwrap();
    let msg: Value = client
        .post(format!("{base}/threads/{thread_id}/messages"))
        .header("maidan-test-member-id", alice_id)
        .json(&json!({"body": "delete me"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let msg_id = msg["id"].as_str().unwrap();
    let uri = format!("maidan://threads/{thread_id}");

    let (notify_tx, mut notify_rx) = tokio::sync::mpsc::channel::<String>(4);
    let sse_client = client.clone();
    let sse_base = base.clone();
    let sse_task = tokio::spawn(async move {
        let resp = sse_client
            .get(format!("{sse_base}/mcp/notifications"))
            .send()
            .await
            .unwrap();
        let mut stream = resp.bytes_stream();
        let mut buf = String::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.unwrap();
            buf.push_str(&String::from_utf8_lossy(&chunk));
            if buf.contains("notifications/resources/updated") {
                let _ = notify_tx.send(buf).await;
                break;
            }
        }
    });

    let subscribe = rpc(
        &client,
        &base,
        20,
        "resources/subscribe",
        json!({ "uri": uri }),
    )
    .await;
    assert!(subscribe["error"].is_null());

    let resp = client
        .delete(format!("{base}/messages/{msg_id}"))
        .header("maidan-test-member-id", alice_id)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);

    let payload = tokio::time::timeout(Duration::from_secs(5), notify_rx.recv())
        .await
        .expect("timed out waiting for tombstone SSE notification")
        .expect("collector exited");
    assert!(payload.contains("notifications/resources/updated"));
    assert!(payload.contains(&uri));

    sse_task.abort();
    server.abort();
}

#[tokio::test]
async fn http_edit_message_emits_resource_updated_sse_notification() {
    let (addr, client, server, _dir) = spawn().await;
    let base = format!("http://{addr}");

    let ws_resp: Value = client
        .post(format!("{base}/workspaces"))
        .json(&json!({"name": "edit-notify"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let workspace_id = ws_resp["id"].as_str().unwrap();
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
        .json(&json!({"title": "t"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let thread_id = th["id"].as_str().unwrap();
    let msg: Value = client
        .post(format!("{base}/threads/{thread_id}/messages"))
        .header("maidan-test-member-id", alice_id)
        .json(&json!({"body": "original"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let msg_id = msg["id"].as_str().unwrap();
    let uri = format!("maidan://threads/{thread_id}");

    let (notify_tx, mut notify_rx) = tokio::sync::mpsc::channel::<String>(4);
    let sse_client = client.clone();
    let sse_base = base.clone();
    let sse_task = tokio::spawn(async move {
        let resp = sse_client
            .get(format!("{sse_base}/mcp/notifications"))
            .send()
            .await
            .unwrap();
        let mut stream = resp.bytes_stream();
        let mut buf = String::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.unwrap();
            buf.push_str(&String::from_utf8_lossy(&chunk));
            if buf.contains("notifications/resources/updated") {
                let _ = notify_tx.send(buf).await;
                break;
            }
        }
    });

    let subscribe = rpc(
        &client,
        &base,
        21,
        "resources/subscribe",
        json!({ "uri": uri }),
    )
    .await;
    assert!(subscribe["error"].is_null());

    let resp = client
        .patch(format!("{base}/messages/{msg_id}"))
        .header("maidan-test-member-id", alice_id)
        .json(&json!({"body": "edited"}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);

    let payload = tokio::time::timeout(Duration::from_secs(5), notify_rx.recv())
        .await
        .expect("timed out waiting for edit SSE notification")
        .expect("collector exited");
    assert!(payload.contains("notifications/resources/updated"));
    assert!(payload.contains(&uri));

    sse_task.abort();
    server.abort();
}

#[tokio::test]
async fn parse_error_for_garbage_body() {
    let (addr, client, server, _dir) = spawn().await;
    let resp = client
        .post(format!("http://{addr}/mcp"))
        .header("content-type", "application/json")
        .body("{not json")
        .send()
        .await
        .unwrap();
    let body: Value = resp.json().await.unwrap();
    assert_eq!(body["error"]["code"], -32700);
    server.abort();
}

#[tokio::test]
async fn json_that_is_not_a_request_is_an_invalid_request_not_a_parse_error() {
    let (addr, client, server, _dir) = spawn().await;
    let post = |path: &'static str, body: &'static str| {
        client
            .post(format!("http://{addr}{path}"))
            .header("content-type", "application/json")
            .header("accept", "application/json")
            .body(body)
            .send()
    };
    for path in ["/mcp", "/mcp/streamable"] {
        for body in [
            r#"{"jsonrpc":"2.0","id":1}"#,
            r#"{"jsonrpc":"2.0","id":null,"method":"tools/list"}"#,
            r#""tools/list""#,
        ] {
            let resp = post(path, body).await.unwrap();
            assert_eq!(resp.status(), StatusCode::OK, "{path} {body}");
            let answer: Value = resp.json().await.unwrap();
            assert_eq!(answer["error"]["code"], -32600, "{path} {body}: {answer}");
            assert_eq!(answer["id"], Value::Null);
        }
    }
    let batch: Value = post("/mcp", r#"[1,{"jsonrpc":"2.0","id":2,"method":"ping"}]"#)
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(batch[0]["error"]["code"], -32600, "{batch}");
    assert_eq!(batch[1]["id"], 2, "{batch}");
    server.abort();
}

#[tokio::test]
async fn mcp_initialize_negotiates_protocol_version() {
    let (addr, client, server, _dir) = spawn().await;
    let base = format!("http://{addr}");
    // A supported requested version is echoed back verbatim.
    let ok = rpc(
        &client,
        &base,
        1,
        "initialize",
        json!({ "protocolVersion": "2024-11-05", "capabilities": {}, "clientInfo": {"name": "t"} }),
    )
    .await;
    assert_eq!(ok["result"]["protocolVersion"], "2024-11-05");
    // An unsupported requested version falls back to the server's default (2026-07-28).
    let fallback = rpc(
        &client,
        &base,
        2,
        "initialize",
        json!({ "protocolVersion": "1999-01-01" }),
    )
    .await;
    assert_eq!(fallback["result"]["protocolVersion"], "2026-07-28");
    server.abort();
}

#[tokio::test]
async fn mcp_batch_returns_array_of_responses() {
    let (addr, client, server, _dir) = spawn().await;
    let batch = json!([
        { "jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {} },
        { "jsonrpc": "2.0", "method": "notifications/initialized" },
        { "jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {} },
    ]);
    let resp = client
        .post(format!("http://{addr}/mcp"))
        .json(&batch)
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let body: Value = resp.json().await.unwrap();
    let arr = body.as_array().expect("batch returns an array");
    // Two id-bearing requests → two responses; the notification produces none.
    assert_eq!(arr.len(), 2);
    let ids: Vec<&Value> = arr.iter().map(|r| &r["id"]).collect();
    assert!(ids.contains(&&json!(1)) && ids.contains(&&json!(2)));
    server.abort();
}

#[tokio::test]
async fn mcp_notification_gets_202_and_no_body() {
    let (addr, client, server, _dir) = spawn().await;
    let resp = client
        .post(format!("http://{addr}/mcp"))
        .json(&json!({ "jsonrpc": "2.0", "method": "notifications/initialized" }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::ACCEPTED);
    assert!(resp.bytes().await.unwrap().is_empty());
    server.abort();
}

#[tokio::test]
async fn mcp_unsupported_protocol_version_header_is_rejected() {
    let (addr, client, server, _dir) = spawn().await;
    let resp = client
        .post(format!("http://{addr}/mcp"))
        .header("mcp-protocol-version", "1999-01-01")
        .json(&json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {} }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    // A supported version passes through.
    let ok = client
        .post(format!("http://{addr}/mcp"))
        .header("mcp-protocol-version", "2024-11-05")
        .json(&json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list", "params": {} }))
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status(), StatusCode::OK);
    server.abort();
}

/// The `enum` a tool publishes is the set its handler accepts: a value it
/// lists goes through, and one outside it is refused as invalid params, not
/// absorbed. An enum narrower than the handler hides a value an agent may
/// need; a wider one invites a call the server refuses.
#[tokio::test]
async fn published_enums_are_the_values_the_server_accepts() {
    let (addr, client, server, _dir) = spawn().await;
    let base = format!("http://{addr}");

    let tools = rpc(&client, &base, 1, "tools/list", json!({})).await;
    let tool = |name: &str| {
        tools["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == name)
            .unwrap_or_else(|| panic!("tools/list has no {name}"))
            .clone()
    };
    let strings = |v: &Value| -> Vec<String> {
        v.as_array()
            .unwrap_or_else(|| panic!("not an enum: {v}"))
            .iter()
            .map(|s| s.as_str().unwrap().to_string())
            .collect()
    };
    let actions =
        strings(&tool("transition_thread")["inputSchema"]["properties"]["action"]["enum"]);
    assert_eq!(actions, ["start_review", "close", "archive"]);
    let block_types = strings(
        &tool("post_message")["inputSchema"]["properties"]["content"]["items"]["properties"]
            ["type"]["enum"],
    );

    let ws: Value = client
        .post(format!("{base}/workspaces"))
        .json(&json!({"name": "enums"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let workspace_id = ws["id"].as_str().unwrap().to_string();
    let alice: Value = client
        .post(format!("{base}/workspaces/{workspace_id}/members"))
        .json(&json!({"handle": "alice", "kind": "agent"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let alice_id = alice["id"].as_str().unwrap().to_string();
    let ch: Value = client
        .post(format!("{base}/workspaces/{workspace_id}/channels"))
        .json(&json!({"name": "work"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let channel_id = ch["id"].as_str().unwrap().to_string();
    let th: Value = client
        .post(format!("{base}/channels/{channel_id}/threads"))
        .json(&json!({"title": "enums"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let thread_id = th["id"].as_str().unwrap().to_string();
    let call = |id: u64, name: &'static str, arguments: Value| {
        let (client, base, alice_id) = (client.clone(), base.clone(), alice_id.clone());
        async move {
            rpc_as(
                &client,
                &base,
                id,
                "tools/call",
                json!({"name": name, "arguments": arguments}),
                &alice_id,
            )
            .await
        }
    };

    let refused = call(
        10,
        "transition_thread",
        json!({"thread_id": thread_id, "action": "reopen"}),
    )
    .await;
    assert_eq!(refused["error"]["code"], -32602, "{refused}");
    assert!(
        refused["error"]["message"]
            .as_str()
            .unwrap()
            .contains("unknown action \"reopen\""),
        "{refused}"
    );

    // Each listed action, in the order the FSM takes them, is accepted.
    for (i, (action, to_state)) in actions
        .iter()
        .zip(["in_review", "closed", "archived"])
        .enumerate()
    {
        let resp = call(
            11 + i as u64,
            "transition_thread",
            json!({"thread_id": thread_id, "action": action}),
        )
        .await;
        assert!(resp["error"].is_null(), "{action}: {resp}");
        let moved = unwrap_tool_text(&resp["result"]);
        assert_eq!(moved["state"], to_state, "{action}: {moved}");
    }

    let open: Value = client
        .post(format!("{base}/channels/{channel_id}/threads"))
        .json(&json!({"title": "blocks"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let open_id = open["id"].as_str().unwrap().to_string();
    let block = |kind: &str| match kind {
        "text" => json!({"type": "text", "text": "hi"}),
        "code" => json!({"type": "code", "language": "rust", "code": "fn main() {}"}),
        "tool_use" => json!({"type": "tool_use", "id": "t1", "name": "grep", "input": {}}),
        "tool_result" => json!({"type": "tool_result", "tool_use_id": "t1", "content": "ok"}),
        "resource_link" => json!({"type": "resource_link", "uri": "maidan://threads/x"}),
        other => panic!("post_message lists a block type this test does not know: {other}"),
    };
    for (i, kind) in block_types.iter().enumerate() {
        let resp = call(
            20 + i as u64,
            "post_message",
            json!({"thread_id": open_id, "body": "", "content": [block(kind)]}),
        )
        .await;
        assert!(resp["error"].is_null(), "{kind}: {resp}");
        let posted = unwrap_tool_text(&resp["result"]);
        assert_eq!(posted["content"][0]["type"], kind.as_str(), "{posted}");
    }
    let refused = call(
        30,
        "post_message",
        json!({"thread_id": open_id, "body": "", "content": [{"type": "image", "url": "x"}]}),
    )
    .await;
    assert_eq!(refused["error"]["code"], -32602, "{refused}");
    // The handler's own list of block types is the published one.
    let message = refused["error"]["message"].as_str().unwrap();
    let mut accepted: Vec<String> = message
        .split("expected one of ")
        .nth(1)
        .unwrap_or_else(|| panic!("no expected list in {message}"))
        .split('`')
        .skip(1)
        .step_by(2)
        .map(str::to_string)
        .collect();
    accepted.sort();
    let mut published = block_types.clone();
    published.sort();
    assert_eq!(published, accepted, "{message}");

    server.abort();
}

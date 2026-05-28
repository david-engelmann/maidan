//! End-to-end MCP test: HTTP POST /mcp drives a full
//! initialize → tools/list → tools/call → resources/read sequence.

use std::{sync::Arc, time::Duration};

use base64::Engine;
use futures::StreamExt;
use maidan_artifacts::LocalFsStore;
use maidan_bus::InMemoryBus;
use maidan_server::{router, AppState};
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
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
    let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool.clone()));
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
    let body = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": method,
        "params": params,
    });
    client
        .post(format!("{base}/mcp"))
        .json(&body)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

fn unwrap_tool_text(result: &Value) -> Value {
    let text = result["content"][0]["text"].as_str().unwrap();
    serde_json::from_str(text).unwrap()
}

#[tokio::test]
async fn full_mcp_flow() {
    let (addr, client, server, _dir) = spawn().await;
    let base = format!("http://{addr}");

    // initialize
    let init = rpc(&client, &base, 1, "initialize", json!({})).await;
    assert_eq!(init["jsonrpc"], "2.0");
    assert_eq!(init["id"], 1);
    assert_eq!(init["result"]["protocolVersion"], "2024-11-05");
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
    let resp = rpc(
        &client,
        &base,
        3,
        "tools/call",
        json!({
            "name": "list_channels",
            "arguments": {"workspace_id": workspace_id}
        }),
    )
    .await;
    let channels = unwrap_tool_text(&resp["result"]);
    assert_eq!(channels.as_array().unwrap().len(), 1);

    // tools/call: post_message
    let resp = rpc(
        &client,
        &base,
        4,
        "tools/call",
        json!({
            "name": "post_message",
            "arguments": {
                "thread_id": thread_id,
                "author_id": alice_id,
                "body": "hi from mcp"
            }
        }),
    )
    .await;
    let posted = unwrap_tool_text(&resp["result"]);
    assert_eq!(posted["body"], "hi from mcp");

    // tools/call: list_messages
    let resp = rpc(
        &client,
        &base,
        5,
        "tools/call",
        json!({
            "name": "list_messages",
            "arguments": {"thread_id": thread_id, "limit": 10}
        }),
    )
    .await;
    let messages = unwrap_tool_text(&resp["result"]);
    let msgs = messages.as_array().unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0]["body"], "hi from mcp");

    // resources/list
    let resources = rpc(&client, &base, 6, "resources/list", json!({})).await;
    let uris: Vec<&str> = resources["result"]["resources"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["uri"].as_str().unwrap())
        .collect();
    assert!(uris.iter().any(|u| u.contains("workspaces")));
    assert!(uris.iter().any(|u| u.contains("threads")));

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
    let resp = rpc(
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

    let _ = rpc(
        &client,
        &base,
        2,
        "tools/call",
        json!({
            "name": "post_message",
            "arguments": {
                "thread_id": thread_id,
                "author_id": alice_id,
                "body": "notify me"
            }
        }),
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

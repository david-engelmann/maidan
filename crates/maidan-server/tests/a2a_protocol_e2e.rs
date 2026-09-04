//! A2A protocol JSON-RPC: SendMessage posts to a thread; GetTask returns the task.

use std::{sync::Arc, time::Duration};

use futures::StreamExt;
use maidan_a2a::{A2aClient, SendMessageRequest};
use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_bus::InMemoryBus;
use maidan_server::FederationRuntime;
use maidan_server::{router, AppState};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    ApprovalGateState, MemberKind, NewApiToken, NewApprovalGate, NewChannel, NewMember, NewThread,
    NewWorkspace,
};
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;
use std::sync::atomic::AtomicI64;

#[tokio::test]
async fn a2a_send_message_posts_to_thread_and_get_task_round_trips() {
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
    let _server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let base = format!("http://{addr}");

    let ws: serde_json::Value = client
        .post(format!("{base}/workspaces"))
        .json(&json!({"name": "a2a-ws"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let workspace_id = ws["id"].as_str().unwrap();

    let member: serde_json::Value = client
        .post(format!("{base}/workspaces/{workspace_id}/members"))
        .json(&json!({"handle": "agent", "kind": "agent"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let author_id = member["id"].as_str().unwrap();

    let ch: serde_json::Value = client
        .post(format!("{base}/workspaces/{workspace_id}/channels"))
        .json(&json!({"name": "general"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let channel_id = ch["id"].as_str().unwrap();

    let th: serde_json::Value = client
        .post(format!("{base}/channels/{channel_id}/threads"))
        .json(&json!({"title": "a2a"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let thread_id = th["id"].as_str().unwrap();

    let a2a = A2aClient::new(&base).unwrap();
    let result = a2a
        .send_message(
            serde_json::from_value::<SendMessageRequest>(json!({
                "message": {
                    "role": "user",
                    "parts": [{ "type": "text", "text": "via a2a" }]
                },
                "metadata": {
                    "maidan": { "threadId": thread_id, "authorId": author_id }
                }
            }))
            .unwrap(),
        )
        .await
        .unwrap();
    let task_id = result["task"]["id"].as_str().expect("task id");
    assert_eq!(
        result["task"]["status"]["state"].as_str(),
        Some("TASK_STATE_COMPLETED")
    );

    let task = a2a.get_task(task_id).await.unwrap();
    assert_eq!(task.id, task_id);

    let listed: Vec<serde_json::Value> = client
        .get(format!("{base}/threads/{thread_id}/messages"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        listed.iter().any(|m| m["body"].as_str() == Some("via a2a")),
        "message body not found"
    );
}

#[tokio::test]
async fn a2a_send_message_preserves_parts_as_structured_content() {
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
    let _server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::new();
    let base = format!("http://{addr}");

    let ws: serde_json::Value = client
        .post(format!("{base}/workspaces"))
        .json(&json!({"name": "a2a-content"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let workspace_id = ws["id"].as_str().unwrap();
    let member: serde_json::Value = client
        .post(format!("{base}/workspaces/{workspace_id}/members"))
        .json(&json!({"handle": "agent", "kind": "agent"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let author_id = member["id"].as_str().unwrap();
    let ch: serde_json::Value = client
        .post(format!("{base}/workspaces/{workspace_id}/channels"))
        .json(&json!({"name": "general"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let channel_id = ch["id"].as_str().unwrap();
    let th: serde_json::Value = client
        .post(format!("{base}/channels/{channel_id}/threads"))
        .json(&json!({"title": "a2a"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let thread_id = th["id"].as_str().unwrap();

    let a2a = A2aClient::new(&base).unwrap();
    a2a.send_message(
        serde_json::from_value::<SendMessageRequest>(json!({
            "message": {
                "role": "user",
                "parts": [
                    { "type": "text", "text": "part one" },
                    { "type": "text", "text": "part two" }
                ]
            },
            "metadata": { "maidan": { "threadId": thread_id, "authorId": author_id } }
        }))
        .unwrap(),
    )
    .await
    .unwrap();

    let listed: Vec<serde_json::Value> = client
        .get(format!("{base}/threads/{thread_id}/messages"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let msg = listed
        .iter()
        .find(|m| m["body"].as_str() == Some("part one\npart two"))
        .expect("a2a message with joined body");
    // The parts are preserved as structured content blocks (Cluster 194).
    let content = msg["content"].as_array().expect("content array");
    assert_eq!(content.len(), 2);
    assert_eq!(content[0], json!({"type": "text", "text": "part one"}));
    assert_eq!(content[1], json!({"type": "text", "text": "part two"}));
}

#[tokio::test]
async fn a2a_send_streaming_message_returns_sse_task_updates() {
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
    let _server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let base = format!("http://{addr}");

    let ws: serde_json::Value = client
        .post(format!("{base}/workspaces"))
        .json(&json!({"name": "a2a-stream-ws"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let workspace_id = ws["id"].as_str().unwrap();

    let member: serde_json::Value = client
        .post(format!("{base}/workspaces/{workspace_id}/members"))
        .json(&json!({"handle": "stream-agent", "kind": "agent"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let author_id = member["id"].as_str().unwrap();

    let ch: serde_json::Value = client
        .post(format!("{base}/workspaces/{workspace_id}/channels"))
        .json(&json!({"name": "general"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let channel_id = ch["id"].as_str().unwrap();

    let th: serde_json::Value = client
        .post(format!("{base}/channels/{channel_id}/threads"))
        .json(&json!({"title": "stream"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let thread_id = th["id"].as_str().unwrap();

    let a2a = A2aClient::new(&base).unwrap();
    let events = a2a
        .send_streaming_message(
            serde_json::from_value::<SendMessageRequest>(json!({
                "message": {
                    "role": "user",
                    "parts": [{ "type": "text", "text": "streamed" }]
                },
                "metadata": {
                    "maidan": { "threadId": thread_id, "authorId": author_id }
                }
            }))
            .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        events.len() >= 2,
        "expected SSE frames, got {}",
        events.len()
    );
    let first = events[0].result.as_ref().expect("first result");
    assert_eq!(
        first["task"]["status"]["state"].as_str(),
        Some("TASK_STATE_WORKING")
    );
    let second = events[1].result.as_ref().expect("second result");
    assert_eq!(
        second["statusUpdate"]["status"]["state"].as_str(),
        Some("TASK_STATE_COMPLETED")
    );
    assert_eq!(second["statusUpdate"]["final"].as_bool(), Some(true));
}

#[tokio::test]
async fn a2a_get_task_loads_from_store_after_send_message() {
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
    let _server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let base = format!("http://{addr}");

    let ws: serde_json::Value = client
        .post(format!("{base}/workspaces"))
        .json(&json!({"name": "a2a-persist"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let workspace_id = ws["id"].as_str().unwrap();

    let member: serde_json::Value = client
        .post(format!("{base}/workspaces/{workspace_id}/members"))
        .json(&json!({"handle": "agent", "kind": "agent"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let author_id = member["id"].as_str().unwrap();

    let ch: serde_json::Value = client
        .post(format!("{base}/workspaces/{workspace_id}/channels"))
        .json(&json!({"name": "general"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let channel_id = ch["id"].as_str().unwrap();

    let th: serde_json::Value = client
        .post(format!("{base}/channels/{channel_id}/threads"))
        .json(&json!({}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let thread_id = th["id"].as_str().unwrap();

    let a2a = A2aClient::new(&base).unwrap();
    let send_resp = a2a
        .send_message(
            serde_json::from_value::<SendMessageRequest>(json!({
                "message": {
                    "role": "user",
                    "parts": [{ "type": "text", "text": "persisted" }]
                },
                "metadata": {
                    "maidan": { "threadId": thread_id, "authorId": author_id }
                }
            }))
            .unwrap(),
        )
        .await
        .unwrap();
    let task_id = send_resp["task"]["id"].as_str().expect("task id");

    let task = a2a.get_task(task_id).await.unwrap();
    assert_eq!(task.id, task_id);
    assert_eq!(task.status.state, "TASK_STATE_COMPLETED");
}

/// Cluster 352 / H12: a pending held gate surfaces as an `input-required` A2A task
/// (id = the gate id), so an external agent can discover it via `tasks/get` +
/// `tasks/list`; resolving the gate makes the task disappear. Runs with auth
/// ENABLED so the bearer's workspace scopes `tasks/list` (a bypass caller has no
/// real workspace).
#[tokio::test]
async fn a2a_pending_gate_surfaces_as_input_required_task() {
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
    let state = AppState::new(
        store.clone(),
        artifacts,
        bus,
        search,
        Arc::new(maidan_search::HashV1Provider),
        false, // auth ENABLED
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let _server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let base = format!("http://{addr}");

    // Set the graph up through the store; mint a bearer scoped to the workspace.
    let ws = store
        .create_workspace(NewWorkspace {
            name: "gate-task".into(),
        })
        .await
        .unwrap();
    let agent = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: "agent".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "general".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: None,
        })
        .await
        .unwrap();
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: agent.id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: vec![
                capability::MESSAGE_POST.to_string(),
                capability::WORKSPACE_READ.to_string(),
            ],
            expires_at: None,
        })
        .await
        .unwrap();
    let token = secret.as_str().to_string();
    let rpc = |method: &str, params: Value| {
        let (client, base, token) = (client.clone(), base.clone(), token.clone());
        let method = method.to_string();
        async move {
            client
                .post(format!("{base}/a2a/v1/rpc"))
                .bearer_auth(&token)
                .json(&json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}))
                .send()
                .await
                .unwrap()
                .json::<Value>()
                .await
                .unwrap()
        }
    };

    // A real per-message task (completes synchronously).
    let thread_id = thread.id.0.to_string();
    let send = rpc(
        "SendMessage",
        json!({
            "message": { "role": "user", "parts": [{ "type": "text", "text": "hi" }] },
            "metadata": { "maidan": { "threadId": thread_id, "authorId": agent.id.0 } }
        }),
    )
    .await;
    let real_task_id = send["result"]["task"]["id"]
        .as_str()
        .expect("task id")
        .to_string();

    // Open a pending held gate on the thread (as `request_approval` would).
    let gate = store
        .create_approval_gate(&NewApprovalGate {
            workspace_id: ws.id,
            thread_id: Some(thread.id),
            requested_by: agent.id,
            prompt: "Deploy to prod?".into(),
            schema: None,
        })
        .await
        .unwrap();
    let gate_id = gate.id.0.to_string();

    // GetTask(gate_id) → the gate as an input-required task.
    let got = rpc("GetTask", json!({ "id": gate_id })).await;
    assert_eq!(got["result"]["id"], json!(gate_id));
    assert_eq!(
        got["result"]["status"]["state"],
        json!("TASK_STATE_INPUT_REQUIRED")
    );
    assert_eq!(got["result"]["contextId"], json!(thread_id));

    // ListTasks (REST §11) leads with the gate-task and still shows the real task.
    let list: Value = client
        .get(format!("{base}/a2a/v1/tasks"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let tasks = list["tasks"].as_array().unwrap();
    assert!(
        tasks.iter().any(|t| t["id"] == json!(gate_id)
            && t["status"]["state"] == json!("TASK_STATE_INPUT_REQUIRED")),
        "the gate is listed as an input-required task"
    );
    assert!(
        tasks.iter().any(|t| t["id"] == json!(real_task_id)),
        "the real per-message task is still listed"
    );

    // Cluster 352.2: `status=input-required` returns exactly the gate; a filter for
    // another state excludes it. `pageSize` is clamped to the spec max of 100.
    let list_ir = |status: &str| {
        let (client, base, token) = (client.clone(), base.clone(), token.clone());
        let status = status.to_string();
        async move {
            client
                .get(format!("{base}/a2a/v1/tasks?status={status}"))
                .bearer_auth(&token)
                .send()
                .await
                .unwrap()
                .json::<Value>()
                .await
                .unwrap()
        }
    };
    let only_gates = list_ir("input-required").await;
    let g = only_gates["tasks"].as_array().unwrap();
    assert_eq!(g.len(), 1, "only the input-required gate matches");
    assert_eq!(g[0]["id"], json!(gate_id));
    let only_done = list_ir("TASK_STATE_COMPLETED").await;
    let d = only_done["tasks"].as_array().unwrap();
    assert!(
        d.iter().any(|t| t["id"] == json!(real_task_id))
            && !d.iter().any(|t| t["id"] == json!(gate_id)),
        "status=completed returns the real task, not the gate"
    );
    let clamped: Value = client
        .get(format!("{base}/a2a/v1/tasks?pageSize=500"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        clamped["pageSize"],
        json!(100),
        "pageSize is clamped to 100"
    );

    // Cluster 352.3: the REST §11 binding uses `application/a2a+json`, not `application/json`.
    let ct_resp = client
        .get(format!("{base}/a2a/v1/tasks"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(
        ct_resp
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok()),
        Some("application/a2a+json"),
        "REST §11 responses carry the A2A media type"
    );

    // Cluster 352.4: `statusTimestampAfter` filters by the task's status timestamp.
    let list_after = |ts: &str| {
        let (client, base, token) = (client.clone(), base.clone(), token.clone());
        let ts = ts.to_string();
        async move {
            client
                .get(format!("{base}/a2a/v1/tasks?statusTimestampAfter={ts}"))
                .bearer_auth(&token)
                .send()
                .await
                .unwrap()
                .json::<Value>()
                .await
                .unwrap()
        }
    };
    let future = list_after("2099-01-01T00:00:00Z").await;
    assert!(
        future["tasks"].as_array().unwrap().is_empty(),
        "nothing changed after a far-future instant"
    );
    let past = list_after("2000-01-01T00:00:00Z").await;
    assert!(
        !past["tasks"].as_array().unwrap().is_empty(),
        "everything changed after a far-past instant"
    );
    let bad = client
        .get(format!(
            "{base}/a2a/v1/tasks?statusTimestampAfter=not-a-date"
        ))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap();
    assert_eq!(
        bad.status(),
        reqwest::StatusCode::BAD_REQUEST,
        "a malformed statusTimestampAfter is rejected"
    );

    // Resolving the gate makes the task disappear (no stale status).
    store
        .resolve_approval_gate(gate.id, agent.id, ApprovalGateState::Accepted, None)
        .await
        .unwrap();
    let gone = rpc("GetTask", json!({ "id": gate_id })).await;
    assert!(
        gone.get("error").is_some(),
        "a resolved gate is no longer a task"
    );
    let list2: Value = client
        .get(format!("{base}/a2a/v1/tasks"))
        .bearer_auth(&token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        !list2["tasks"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["id"] == json!(gate_id)),
        "the resolved gate is gone from the list"
    );
}

#[tokio::test]
async fn a2a_subscribe_to_task_rejects_terminal_task() {
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

    let ws = store
        .create_workspace(maidan_types::NewWorkspace { name: "sub".into() })
        .await
        .unwrap();
    let task = serde_json::json!({
        "id": "done-task",
        "status": { "state": "TASK_STATE_COMPLETED" }
    });
    store
        .upsert_a2a_task(ws.id, "done-task", task)
        .await
        .unwrap();

    let app = router(AppState::for_tests(store, artifacts, bus, search));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let _server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let client = reqwest::Client::new();
    let resp: serde_json::Value = client
        .post(format!("http://{addr}/a2a/v1/rpc"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "SubscribeToTask",
            "params": { "id": "done-task" }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(resp["error"]["code"], -32005);
}

#[tokio::test]
async fn a2a_subscribe_to_task_streams_working_task() {
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

    let ws = store
        .create_workspace(maidan_types::NewWorkspace {
            name: "sub2".into(),
        })
        .await
        .unwrap();
    store
        .upsert_a2a_task(
            ws.id,
            "work-task",
            serde_json::json!({
                "id": "work-task",
                "status": { "state": "TASK_STATE_WORKING" }
            }),
        )
        .await
        .unwrap();

    let app = router(AppState::for_tests(store, artifacts, bus, search));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let _server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let client = reqwest::Client::new();
    let resp = client
        .post(format!("http://{addr}/a2a/v1/rpc"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "SubscribeToTask",
            "params": { "id": "work-task" }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let body = resp.text().await.unwrap();
    assert!(body.contains("TASK_STATE_WORKING"));
    assert!(body.contains("work-task"));
}

#[tokio::test]
async fn a2a_tasks_cancel_marks_working_task_canceled() {
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

    let ws = store
        .create_workspace(maidan_types::NewWorkspace {
            name: "cancel-ws".into(),
        })
        .await
        .unwrap();
    store
        .upsert_a2a_task(
            ws.id,
            "cancel-me",
            serde_json::json!({
                "id": "cancel-me",
                "status": { "state": "TASK_STATE_WORKING" }
            }),
        )
        .await
        .unwrap();

    let app = router(AppState::for_tests(store, artifacts, bus, search));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let _server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let client = reqwest::Client::new();
    let resp: serde_json::Value = client
        .post(format!("http://{addr}/a2a/v1/rpc"))
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "CancelTask",
            "params": { "id": "cancel-me" }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(resp["result"]["status"]["state"], "TASK_STATE_CANCELED");

    let resp2: serde_json::Value = client
        .post(format!("http://{addr}/a2a/v1/rpc"))
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "SubscribeToTask",
            "params": { "id": "cancel-me" }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(resp2["error"]["code"], -32005);
}

#[tokio::test]
async fn a2a_subscribe_to_task_emits_progress_when_task_becomes_terminal() {
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

    let ws = store
        .create_workspace(maidan_types::NewWorkspace {
            name: "progress-ws".into(),
        })
        .await
        .unwrap();
    store
        .upsert_a2a_task(
            ws.id,
            "progress-task",
            serde_json::json!({
                "id": "progress-task",
                "status": { "state": "TASK_STATE_WORKING" }
            }),
        )
        .await
        .unwrap();

    let store_bg = store.clone();
    let ws_id = ws.id;
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(250)).await;
        store_bg
            .upsert_a2a_task(
                ws_id,
                "progress-task",
                serde_json::json!({
                    "id": "progress-task",
                    "status": { "state": "TASK_STATE_COMPLETED" }
                }),
            )
            .await
            .unwrap();
    });

    let app = router(AppState::for_tests(store, artifacts, bus, search));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let _server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap();
    let resp = client
        .post(format!("http://{addr}/a2a/v1/rpc"))
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "SubscribeToTask",
            "params": { "id": "progress-task" }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    let mut buf = String::new();
    let mut stream = resp.bytes_stream();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    while tokio::time::Instant::now() < deadline {
        let Some(chunk) = stream.next().await else {
            break;
        };
        buf.push_str(&String::from_utf8_lossy(&chunk.unwrap()));
        if buf.contains("TASK_STATE_COMPLETED") && buf.contains("statusUpdate") {
            break;
        }
    }
    assert!(
        buf.contains("TASK_STATE_WORKING"),
        "expected initial task frame"
    );
    assert!(
        buf.contains("statusUpdate"),
        "expected progress frame, got: {buf}"
    );
    assert!(buf.contains("TASK_STATE_COMPLETED"));
}

/// Cluster 284: per-task push notification configs — Create/Get/List/Delete over
/// JSON-RPC against a real task created by SendMessage.
#[tokio::test]
async fn a2a_task_push_config_create_get_list_delete() {
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
    let _server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::new();
    let base = format!("http://{addr}");

    // workspace / member / channel / thread
    let ws: Value = client
        .post(format!("{base}/workspaces"))
        .json(&json!({"name": "pc"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let workspace_id = ws["id"].as_str().unwrap();
    let member: Value = client
        .post(format!("{base}/workspaces/{workspace_id}/members"))
        .json(&json!({"handle": "agent", "kind": "agent"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let author_id = member["id"].as_str().unwrap();
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
        .json(&json!({"title": "pc"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let thread_id = th["id"].as_str().unwrap();

    let rpc = |method: &'static str, params: Value| {
        let client = client.clone();
        let base = base.clone();
        async move {
            client
                .post(format!("{base}/a2a/v1/rpc"))
                .json(&json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params}))
                .send()
                .await
                .unwrap()
                .json::<Value>()
                .await
                .unwrap()
        }
    };

    // Create a task via SendMessage.
    let sent = rpc(
        "SendMessage",
        json!({
            "message": {"role": "user", "parts": [{"type": "text", "text": "hi"}]},
            "metadata": {"maidan": {"threadId": thread_id, "authorId": author_id}}
        }),
    )
    .await;
    let task_id = sent["result"]["task"]["id"].as_str().unwrap().to_string();

    // Create a push config (server generates the id).
    let created = rpc(
        "CreateTaskPushNotificationConfig",
        json!({"taskId": task_id, "url": "https://hook.example/a"}),
    )
    .await;
    let config_id = created["result"]["id"].as_str().unwrap().to_string();
    assert_eq!(created["result"]["taskId"].as_str(), Some(task_id.as_str()));
    assert_eq!(
        created["result"]["url"].as_str(),
        Some("https://hook.example/a")
    );

    // Get it back.
    let got = rpc(
        "GetTaskPushNotificationConfig",
        json!({"taskId": task_id, "id": config_id}),
    )
    .await;
    assert_eq!(
        got["result"]["url"].as_str(),
        Some("https://hook.example/a")
    );

    // List: exactly one.
    let listed = rpc(
        "ListTaskPushNotificationConfigs",
        json!({"taskId": task_id}),
    )
    .await;
    assert_eq!(listed["result"]["configs"].as_array().unwrap().len(), 1);

    // Delete it, then the list is empty and a re-get errors.
    let deleted = rpc(
        "DeleteTaskPushNotificationConfig",
        json!({"taskId": task_id, "id": config_id}),
    )
    .await;
    assert!(deleted["result"].is_object());
    let listed2 = rpc(
        "ListTaskPushNotificationConfigs",
        json!({"taskId": task_id}),
    )
    .await;
    assert_eq!(listed2["result"]["configs"].as_array().unwrap().len(), 0);
    let missing = rpc(
        "GetTaskPushNotificationConfig",
        json!({"taskId": task_id, "id": config_id}),
    )
    .await;
    assert!(
        missing["error"].is_object(),
        "get after delete should error"
    );
}

/// Cluster 285: the public Agent Card is A2A v1.0 spec-shaped (§4.4.1).
#[tokio::test]
async fn agent_card_is_spec_shaped() {
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
    let _server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::new();
    let base = format!("http://{addr}");

    let card: Value = client
        .get(format!("{base}/.well-known/agent-card.json"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    // Required §4.4.1 fields.
    assert_eq!(card["name"].as_str(), Some("maidan"));
    assert!(card["description"].as_str().is_some_and(|d| !d.is_empty()));
    assert!(card["version"].as_str().is_some());
    assert!(card["provider"]["organization"].as_str().is_some());
    // supportedInterfaces: first entry is the preferred JSON-RPC binding.
    let iface = &card["supportedInterfaces"][0];
    assert_eq!(iface["protocolBinding"].as_str(), Some("JSONRPC"));
    assert_eq!(iface["protocolVersion"].as_str(), Some("1.0"));
    assert!(iface["url"].as_str().is_some());
    // capabilities object.
    assert_eq!(card["capabilities"]["streaming"].as_bool(), Some(true));
    assert_eq!(
        card["capabilities"]["pushNotifications"].as_bool(),
        Some(true)
    );
    assert_eq!(
        card["capabilities"]["extendedAgentCard"].as_bool(),
        Some(true)
    );
    // input/output modes + skills are non-empty.
    assert!(card["defaultInputModes"]
        .as_array()
        .is_some_and(|m| !m.is_empty()));
    assert!(card["defaultOutputModes"]
        .as_array()
        .is_some_and(|m| !m.is_empty()));
    assert!(card["skills"].as_array().is_some_and(|s| !s.is_empty()));
}

/// Cluster 286: the HTTP+JSON/REST binding (§11) maps the same operations as the
/// JSON-RPC endpoint. Exercises message:send → get → list → push-config CRUD →
/// extendedAgentCard, and confirms the tasks/{id}:cancel custom method routes.
#[tokio::test]
async fn a2a_rest_binding_maps_operations() {
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
    let _server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::new();
    let base = format!("http://{addr}");

    let ws: Value = client
        .post(format!("{base}/workspaces"))
        .json(&json!({"name": "rest"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let wid = ws["id"].as_str().unwrap();
    let member: Value = client
        .post(format!("{base}/workspaces/{wid}/members"))
        .json(&json!({"handle": "a", "kind": "agent"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let author = member["id"].as_str().unwrap();
    let ch: Value = client
        .post(format!("{base}/workspaces/{wid}/channels"))
        .json(&json!({"name": "general"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let cid = ch["id"].as_str().unwrap();
    let th: Value = client
        .post(format!("{base}/channels/{cid}/threads"))
        .json(&json!({"title": "rest"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let tid = th["id"].as_str().unwrap();

    // message:send (REST) → 200 with a task.
    let sent = client
        .post(format!("{base}/a2a/v1/message:send"))
        .json(&json!({
            "message": {"role": "user", "parts": [{"type": "text", "text": "hi"}]},
            "metadata": {"maidan": {"threadId": tid, "authorId": author}}
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(sent.status(), reqwest::StatusCode::OK);
    let sent: Value = sent.json().await.unwrap();
    let task_id = sent["task"]["id"].as_str().unwrap().to_string();

    // get task.
    let got = client
        .get(format!("{base}/a2a/v1/tasks/{task_id}"))
        .send()
        .await
        .unwrap();
    assert_eq!(got.status(), reqwest::StatusCode::OK);
    assert_eq!(
        got.json::<Value>().await.unwrap()["id"].as_str(),
        Some(task_id.as_str())
    );

    // list tasks: the REST route maps to ListTasks and returns the response shape.
    // (Non-empty contents under RBAC are proven by the auth-enabled test in
    // channel_access_e2e; this bypass server has no single workspace to scope by.)
    let listed_resp = client
        .get(format!("{base}/a2a/v1/tasks"))
        .send()
        .await
        .unwrap();
    assert_eq!(listed_resp.status(), reqwest::StatusCode::OK);
    let listed: Value = listed_resp.json().await.unwrap();
    assert!(listed["tasks"].is_array());

    // push-config create → get → list → delete.
    let created: Value = client
        .post(format!(
            "{base}/a2a/v1/tasks/{task_id}/pushNotificationConfigs"
        ))
        .json(&json!({"url": "https://hook.example/x"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let config_id = created["id"].as_str().unwrap().to_string();
    assert_eq!(created["url"].as_str(), Some("https://hook.example/x"));
    let got_pc = client
        .get(format!(
            "{base}/a2a/v1/tasks/{task_id}/pushNotificationConfigs/{config_id}"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(got_pc.status(), reqwest::StatusCode::OK);
    let list_pc: Value = client
        .get(format!(
            "{base}/a2a/v1/tasks/{task_id}/pushNotificationConfigs"
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(list_pc["configs"].as_array().unwrap().len(), 1);
    let del = client
        .delete(format!(
            "{base}/a2a/v1/tasks/{task_id}/pushNotificationConfigs/{config_id}"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(del.status(), reqwest::StatusCode::OK);

    // extendedAgentCard (REST) → spec-shaped card.
    let card: Value = client
        .get(format!("{base}/a2a/v1/extendedAgentCard"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(card["name"].as_str(), Some("maidan"));

    // tasks/{id}:cancel custom method routes (200 or a mapped 4xx, never a 404 route-miss).
    let cancel = client
        .post(format!("{base}/a2a/v1/tasks/{task_id}:cancel"))
        .send()
        .await
        .unwrap();
    assert_ne!(
        cancel.status(),
        reqwest::StatusCode::NOT_FOUND,
        "cancel custom-method route should match"
    );
}

/// Cluster 288: with a public origin + advertised gRPC address configured, the
/// Agent Card advertises absolute HTTP interface URLs and a GRPC interface (§5.2).
#[tokio::test]
async fn agent_card_advertises_configured_transports() {
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
    let mut state = AppState::for_tests(store, artifacts, bus, search);
    state.a2a_card = maidan_server::a2a_agent::A2aCardConfig {
        public_origin: Some("https://maidan.example".into()),
        grpc_public_addr: Some("grpc.maidan.example:443".into()),
    };
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let _server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::new();
    let base = format!("http://{addr}");

    let card: Value = client
        .get(format!("{base}/.well-known/agent-card.json"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let ifaces = card["supportedInterfaces"].as_array().unwrap();
    // HTTP interfaces are absolute (origin-prefixed).
    let jsonrpc = ifaces
        .iter()
        .find(|i| i["protocolBinding"] == "JSONRPC")
        .unwrap();
    assert_eq!(
        jsonrpc["url"].as_str(),
        Some("https://maidan.example/a2a/v1/rpc")
    );
    let rest = ifaces
        .iter()
        .find(|i| i["protocolBinding"] == "HTTP+JSON")
        .unwrap();
    assert_eq!(rest["url"].as_str(), Some("https://maidan.example/a2a/v1"));
    // The gRPC interface is advertised at the configured address.
    let grpc = ifaces
        .iter()
        .find(|i| i["protocolBinding"] == "GRPC")
        .expect("grpc interface advertised");
    assert_eq!(grpc["url"].as_str(), Some("grpc.maidan.example:443"));
    assert_eq!(grpc["protocolVersion"].as_str(), Some("1.0"));
}

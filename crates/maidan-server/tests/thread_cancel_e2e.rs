//! A producer cancels a task it created, over REST and MCP: `workspace:write`
//! lets the creator, anyone else needs `thread:transition` as well, the
//! worker holding the claim is refused its next renew, a repeat answers with
//! the first cancel, and another tenant cannot reach the task.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_bus::InMemoryBus;
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    ChannelId, EventKind, MemberId, MemberKind, NewApiToken, NewChannel, NewMember, NewWorkspace,
    ThreadId, ThreadState, WorkspaceId,
};
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

const PRODUCER: &[&str] = &[capability::WORKSPACE_READ, capability::WORKSPACE_WRITE];
const WORKER: &[&str] = &[capability::WORKSPACE_READ, capability::THREAD_TRANSITION];

async fn spawn() -> (String, Arc<dyn Store>) {
    let pool = SqlitePoolOptions::new()
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
    let state = AppState::new(
        store.clone(),
        Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path())),
        Arc::new(InMemoryBus::new()),
        search,
        Arc::new(maidan_search::HashV1Provider),
        false,
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let app = router(state);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), store)
}

async fn room(store: &dyn Store, name: &str) -> (WorkspaceId, ChannelId) {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap();
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "work".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    (ws.id, channel.id)
}

async fn member_with(
    store: &dyn Store,
    ws: WorkspaceId,
    handle: &str,
    capabilities: &[&str],
) -> (MemberId, String) {
    let member = store
        .create_member(NewMember {
            workspace_id: ws,
            handle: handle.into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws,
            member_id: member.id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: capabilities.iter().map(|c| (*c).to_string()).collect(),
            expires_at: None,
        })
        .await
        .unwrap();
    (member.id, secret.as_str().to_string())
}

async fn tool(base: &str, token: &str, name: &str, arguments: Value) -> Value {
    reqwest::Client::new()
        .post(format!("{base}/mcp"))
        .bearer_auth(token)
        .header("accept", "application/json")
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": name, "arguments": arguments }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

fn tool_result(response: &Value) -> Value {
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("not a tool result: {response}"));
    serde_json::from_str(text).unwrap()
}

fn tool_error(response: &Value) -> String {
    response["error"]["message"]
        .as_str()
        .unwrap_or_else(|| panic!("not a tool error: {response}"))
        .to_string()
}

async fn cancel(base: &str, token: &str, thread: &str, reason: &str) -> reqwest::Response {
    reqwest::Client::new()
        .post(format!("{base}/threads/{thread}/cancel"))
        .bearer_auth(token)
        .json(&json!({ "reason": reason }))
        .send()
        .await
        .unwrap()
}

#[tokio::test]
async fn a_producer_cancels_its_claimed_task_over_http() {
    let (base, store) = spawn().await;
    let (ws, channel) = room(store.as_ref(), "ours").await;
    let (producer_id, producer) = member_with(store.as_ref(), ws, "producer", PRODUCER).await;
    let (_, worker) = member_with(store.as_ref(), ws, "worker", WORKER).await;
    let (_, bystander) = member_with(store.as_ref(), ws, "bystander", PRODUCER).await;
    let (_, reader) =
        member_with(store.as_ref(), ws, "reader", &[capability::WORKSPACE_READ]).await;
    let http = reqwest::Client::new();

    let created: Value = http
        .post(format!("{base}/channels/{}/threads", channel.0))
        .bearer_auth(&producer)
        .json(&json!({ "title": "port the bridge" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(created["created_by"], json!(producer_id.0));
    let thread = created["id"].as_str().unwrap().to_string();

    let claimed = tool_result(
        &tool(
            &base,
            &worker,
            "claim_next_thread",
            json!({ "channel_id": channel.0, "lease_secs": 600 }),
        )
        .await,
    );
    let claimed = if claimed["thread"].is_object() {
        claimed["thread"].clone()
    } else {
        claimed
    };
    assert_eq!(claimed["id"].as_str(), Some(thread.as_str()));
    let lease = claimed["claim_lease_id"].as_str().unwrap().to_string();
    let renew = |token: String| {
        let http = http.clone();
        let url = format!("{base}/threads/{thread}/claim/renew");
        let lease = lease.clone();
        async move {
            http.post(url)
                .bearer_auth(token)
                .json(&json!({ "claim_lease_id": lease, "lease_secs": 600 }))
                .send()
                .await
                .unwrap()
                .status()
        }
    };
    assert_eq!(renew(worker.clone()).await, StatusCode::OK);

    let missing = cancel(&base, &reader, &thread, "no").await;
    assert_eq!(missing.status(), StatusCode::FORBIDDEN);
    assert!(missing
        .text()
        .await
        .unwrap()
        .contains("missing capability: workspace:write"));
    let not_creator = cancel(&base, &bystander, &thread, "not mine").await;
    assert_eq!(not_creator.status(), StatusCode::FORBIDDEN);
    assert_eq!(
        cancel(&base, &producer, &thread, "   ").await.status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        cancel(&base, &producer, &thread, &"x".repeat(501))
            .await
            .status(),
        StatusCode::BAD_REQUEST
    );
    let still = store
        .get_thread(ThreadId(thread.parse().unwrap()))
        .await
        .unwrap();
    assert_eq!(still.state, ThreadState::Open);
    assert!(still.assignee_id.is_some(), "a refused cancel ends nothing");

    let done = cancel(&base, &producer, &thread, "superseded by the new plan").await;
    assert_eq!(done.status(), StatusCode::OK);
    let done: Value = done.json().await.unwrap();
    assert_eq!(done["thread"]["state"], "closed");
    assert_eq!(done["thread"]["assignee_id"], Value::Null);
    assert_eq!(done["cancellation"]["reason"], "superseded by the new plan");
    assert_eq!(done["cancellation"]["cancelled_by"], json!(producer_id.0));

    assert_eq!(renew(worker.clone()).await, StatusCode::NOT_FOUND);

    let again = cancel(&base, &producer, &thread, "a second reason").await;
    assert_eq!(again.status(), StatusCode::OK);
    let again: Value = again.json().await.unwrap();
    assert_eq!(
        again["cancellation"]["reason"],
        "superseded by the new plan"
    );

    let thread_id = ThreadId(thread.parse().unwrap());
    let messages = store.list_messages(thread_id, 10).await.unwrap();
    assert_eq!(messages.len(), 1, "one notice, however many cancels");
    assert_eq!(messages[0].body, "Cancelled: superseded by the new plan");
    let kinds: Vec<EventKind> = store
        .list_thread_events_through(thread_id, i64::MAX)
        .await
        .unwrap()
        .iter()
        .map(|e| e.kind)
        .collect();
    assert_eq!(
        kinds
            .iter()
            .filter(|k| **k == EventKind::ThreadCancelled)
            .count(),
        1
    );

    // Closed through the transition path, not cancelled: nothing to cancel.
    let finished: Value = http
        .post(format!("{base}/channels/{}/threads", channel.0))
        .bearer_auth(&producer)
        .json(&json!({ "title": "landed" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let finished = finished["id"].as_str().unwrap().to_string();
    for action in ["start_review", "close"] {
        let moved = http
            .post(format!("{base}/threads/{finished}"))
            .bearer_auth(&worker)
            .json(&json!({ "action": action }))
            .send()
            .await
            .unwrap();
        assert_eq!(moved.status(), StatusCode::OK, "{action}");
    }
    assert_eq!(
        cancel(&base, &producer, &finished, "too late")
            .await
            .status(),
        StatusCode::CONFLICT
    );
}

/// Two tenants: a member of another workspace, even with
/// `thread:transition`, cannot reach the task, and the task is untouched.
#[tokio::test]
async fn another_tenant_cannot_cancel_a_task() {
    let (base, store) = spawn().await;
    let (ours, channel) = room(store.as_ref(), "ours").await;
    let (theirs, _) = room(store.as_ref(), "theirs").await;
    let (_, producer) = member_with(store.as_ref(), ours, "producer", PRODUCER).await;
    let (_, outsider) = member_with(
        store.as_ref(),
        theirs,
        "outsider",
        &[
            capability::WORKSPACE_READ,
            capability::WORKSPACE_WRITE,
            capability::THREAD_TRANSITION,
        ],
    )
    .await;
    let created: Value = reqwest::Client::new()
        .post(format!("{base}/channels/{}/threads", channel.0))
        .bearer_auth(&producer)
        .json(&json!({ "title": "ours" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let thread = created["id"].as_str().unwrap().to_string();

    let crossing = cancel(&base, &outsider, &thread, "crossing").await;
    assert!(
        matches!(
            crossing.status(),
            StatusCode::NOT_FOUND | StatusCode::FORBIDDEN
        ),
        "{}",
        crossing.status()
    );
    let refused = tool(
        &base,
        &outsider,
        "cancel_thread",
        json!({ "thread_id": thread, "reason": "crossing" }),
    )
    .await;
    assert!(refused["error"].is_object(), "{refused}");

    let thread_id = ThreadId(thread.parse().unwrap());
    assert_eq!(
        store.get_thread(thread_id).await.unwrap().state,
        ThreadState::Open
    );
    assert!(store
        .get_thread_unclaimable(thread_id)
        .await
        .unwrap()
        .is_none());
    assert!(store.list_messages(thread_id, 10).await.unwrap().is_empty());
}

#[tokio::test]
async fn a_producer_cancels_its_task_over_mcp() {
    let (base, store) = spawn().await;
    let (ws, channel) = room(store.as_ref(), "ours").await;
    let (producer_id, producer) = member_with(store.as_ref(), ws, "producer", PRODUCER).await;
    let (_, bystander) = member_with(store.as_ref(), ws, "bystander", PRODUCER).await;
    let (lead_id, lead) = member_with(
        store.as_ref(),
        ws,
        "lead",
        &[
            capability::WORKSPACE_READ,
            capability::WORKSPACE_WRITE,
            capability::THREAD_TRANSITION,
        ],
    )
    .await;

    let created = tool_result(
        &tool(
            &base,
            &producer,
            "create_thread",
            json!({ "channel_id": channel.0, "title": "made over mcp" }),
        )
        .await,
    );
    assert_eq!(created["created_by"], json!(producer_id.0));
    let thread = created["id"].as_str().unwrap().to_string();

    let refused = tool(
        &base,
        &bystander,
        "cancel_thread",
        json!({ "thread_id": thread, "reason": "not mine" }),
    )
    .await;
    assert!(
        tool_error(&refused).contains("only the member that created this thread"),
        "{refused}"
    );
    let blank = tool(
        &base,
        &producer,
        "cancel_thread",
        json!({ "thread_id": thread, "reason": " " }),
    )
    .await;
    assert!(tool_error(&blank).contains("reason"), "{blank}");

    let done = tool_result(
        &tool(
            &base,
            &producer,
            "cancel_thread",
            json!({ "thread_id": thread, "reason": "no longer needed" }),
        )
        .await,
    );
    assert_eq!(done["thread"]["state"], "closed");
    assert_eq!(done["cancellation"]["cancelled_by"], json!(producer_id.0));
    let again = tool_result(
        &tool(
            &base,
            &producer,
            "cancel_thread",
            json!({ "thread_id": thread, "reason": "again" }),
        )
        .await,
    );
    assert_eq!(again["cancellation"]["reason"], "no longer needed");

    // A `thread:transition` holder cancels a task someone else created.
    let other = tool_result(
        &tool(
            &base,
            &producer,
            "create_thread",
            json!({ "channel_id": channel.0, "title": "the lead drops it" }),
        )
        .await,
    );
    let dropped = tool_result(
        &tool(
            &base,
            &lead,
            "cancel_thread",
            json!({ "thread_id": other["id"], "reason": "dropped" }),
        )
        .await,
    );
    assert_eq!(dropped["cancellation"]["cancelled_by"], json!(lead_id.0));
}

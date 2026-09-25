//! A change request sends work back, over REST and MCP with auth on. A worker
//! claims a task, hands it to review and lets go; a reviewer requests changes
//! with a note; the thread is `open` again, the note is in its context, the
//! worker can claim it back, and the reopen is in the event log. The
//! transition route does not take `request_changes` itself: the review, which
//! carries the note, is the only way to send work back.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{capability, hash_secret, TokenSecret};
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberId, MemberKind, NewApiToken, NewChannel, NewMember, NewThread, NewWorkspace, WorkspaceId,
};
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

async fn mint(store: &dyn Store, ws: WorkspaceId, member: MemberId, caps: Vec<String>) -> String {
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws,
            member_id: member,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: caps,
            expires_at: None,
        })
        .await
        .unwrap();
    secret.as_str().to_string()
}

async fn spawn() -> (SocketAddr, reqwest::Client, Arc<dyn Store>) {
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
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
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
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, reqwest::Client::new(), store)
}

async fn call(
    client: &reqwest::Client,
    method: reqwest::Method,
    url: String,
    token: &str,
    body: Option<Value>,
) -> (StatusCode, Value) {
    let mut req = client.request(method, url).header("Authorization", token);
    if let Some(body) = body {
        req = req.json(&body);
    }
    let resp = req.send().await.unwrap();
    let status = resp.status();
    let text = resp.text().await.unwrap();
    (
        status,
        serde_json::from_str(&text).unwrap_or(Value::String(text)),
    )
}

#[tokio::test]
async fn a_change_request_sends_work_back_over_rest_and_mcp() {
    let (addr, client, store) = spawn().await;
    let base = format!("http://{addr}");
    let ws = store
        .create_workspace(NewWorkspace { name: "rc".into() })
        .await
        .unwrap();
    let member = |handle: &'static str| {
        let store = store.clone();
        let ws = ws.id;
        async move {
            store
                .create_member(NewMember {
                    workspace_id: ws,
                    handle: handle.into(),
                    display_name: None,
                    kind: MemberKind::Agent,
                })
                .await
                .unwrap()
        }
    };
    let worker = member("worker").await;
    let reviewer = member("reviewer").await;
    let caps = vec![
        capability::THREAD_TRANSITION.into(),
        capability::WORKSPACE_READ.into(),
        capability::MESSAGE_POST.into(),
    ];
    let worker_h = format!(
        "Bearer {}",
        mint(store.as_ref(), ws.id, worker.id, caps.clone()).await
    );
    let reviewer_h = format!(
        "Bearer {}",
        mint(store.as_ref(), ws.id, reviewer.id, caps).await
    );
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "work".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let cid = channel.id.0;
    use reqwest::Method;

    // The worker takes the task, hands it to review, lets go.
    let hand_in = |tid: uuid::Uuid| {
        let client = client.clone();
        let base = base.clone();
        let worker_h = worker_h.clone();
        async move {
            let (s, claim) = call(
                &client,
                Method::POST,
                format!("{base}/channels/{cid}/threads/claim-next"),
                &worker_h,
                Some(json!({"lease_secs": 60})),
            )
            .await;
            assert_eq!(s, StatusCode::OK, "{claim}");
            assert_eq!(claim["id"], json!(tid));
            let lease = claim["claim_lease_id"].clone();
            let (s, _) = call(
                &client,
                Method::POST,
                format!("{base}/threads/{tid}"),
                &worker_h,
                Some(json!({"action": "start_review"})),
            )
            .await;
            assert_eq!(s, StatusCode::OK);
            let (s, _) = call(
                &client,
                Method::POST,
                format!("{base}/threads/{tid}/claim/release"),
                &worker_h,
                Some(json!({"claim_lease_id": lease})),
            )
            .await;
            assert_eq!(s, StatusCode::OK);
        }
    };
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("task".into()),
        })
        .await
        .unwrap();
    let tid = thread.id.0;
    hand_in(tid).await;

    // The transition route does not send work back; it says how.
    let (s, refused) = call(
        &client,
        Method::POST,
        format!("{base}/threads/{tid}"),
        &reviewer_h,
        Some(json!({"action": "request_changes"})),
    )
    .await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(refused.to_string().contains("submit a review"), "{refused}");

    // The reviewer sends it back with a note.
    let (s, review) = call(
        &client,
        Method::POST,
        format!("{base}/threads/{tid}/reviews"),
        &reviewer_h,
        Some(json!({"decision": "request_changes", "note": "cover the empty input"})),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "{review}");
    let (_, t) = call(
        &client,
        Method::GET,
        format!("{base}/threads/{tid}"),
        &worker_h,
        None,
    )
    .await;
    assert_eq!(t["state"], "open");

    // The worker sees why, and can take it back.
    let (_, ctx) = call(
        &client,
        Method::GET,
        format!("{base}/threads/{tid}/context"),
        &worker_h,
        None,
    )
    .await;
    assert_eq!(
        ctx["change_requests"][0]["note"], "cover the empty input",
        "{ctx}"
    );
    assert_eq!(
        ctx["change_requests"][0]["reviewer_id"],
        json!(reviewer.id.0)
    );
    let (_, again) = call(
        &client,
        Method::POST,
        format!("{base}/channels/{cid}/threads/claim-next"),
        &worker_h,
        Some(json!({"lease_secs": 60})),
    )
    .await;
    assert_eq!(again["id"], json!(tid));

    // The reopen is in the log, attributed to the reviewer.
    let (_, events) = call(
        &client,
        Method::GET,
        format!(
            "{base}/workspaces/{}/events?types=thread_state_changed",
            ws.id.0
        ),
        &worker_h,
        None,
    )
    .await;
    let reopen = events
        .as_array()
        .expect("event list")
        .iter()
        .find(|e| e["payload"]["to_state"] == "open")
        .unwrap_or_else(|| panic!("no reopen event in {events}"));
    assert_eq!(reopen["payload"]["actor_id"], json!(reviewer.id.0));

    // The same over MCP, on a second task.
    let second = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("second".into()),
        })
        .await
        .unwrap();
    // Let go of the first one so the second is next.
    let (_, _) = call(
        &client,
        Method::POST,
        format!("{base}/threads/{tid}/claim/release"),
        &worker_h,
        Some(json!({"claim_lease_id": again["claim_lease_id"]})),
    )
    .await;
    let (_, _) = call(
        &client,
        Method::POST,
        format!("{base}/threads/{tid}"),
        &worker_h,
        Some(json!({"action": "start_review"})),
    )
    .await;
    hand_in(second.id.0).await;
    let resp = client
        .post(format!("{base}/mcp/streamable"))
        .header("Authorization", &reviewer_h)
        .header("mcp-protocol-version", "2026-07-28")
        .header("accept", "application/json")
        .json(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {
            "name": "submit_review",
            "arguments": {"thread_id": second.id.0, "decision": "request_changes", "note": "again"}
        }}))
        .send()
        .await
        .unwrap();
    let rpc: Value = resp.json().await.unwrap();
    assert!(rpc["result"]["isError"] != json!(true), "{rpc}");
    assert_eq!(
        store.get_thread(second.id).await.unwrap().state,
        maidan_types::ThreadState::Open
    );
}

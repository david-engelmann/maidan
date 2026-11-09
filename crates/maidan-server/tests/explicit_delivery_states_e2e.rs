//! A call aimed at a target that is gone, ended or reaped answers an error
//! the caller sees, never a success (Open Work research row 13). Over REST
//! that is a non-2xx problem; over MCP a JSON-RPC error with its own code and
//! a `data.code` a client can match on:
//!
//! - a renewal or result on a lease that expired, before anything reaped it,
//!   is `409 claim-lost` / `-32012`;
//! - a message to an archived thread is `409 thread-ended` / `-32013`, to a
//!   deleted one `410 gone` / `-32014`, and to a closed one still `201`;
//! - a footprint or a required skill on a closed thread is `409
//!   thread-ended`, while an open thread nobody holds takes a footprint.

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
    ChannelId, MemberId, MemberKind, NewApiToken, NewChannel, NewMember, NewWorkspace, ThreadId,
    WorkspaceId,
};
use reqwest::StatusCode;
use serde_json::{json, Value};

const AGENT: &[&str] = &[
    capability::WORKSPACE_READ,
    capability::WORKSPACE_WRITE,
    capability::MESSAGE_POST,
    capability::THREAD_TRANSITION,
];

struct Room {
    base: String,
    store: Arc<dyn Store>,
    pool: sqlx::SqlitePool,
    channel: ChannelId,
    member: MemberId,
    token: String,
}

async fn room() -> Room {
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
    let search: Arc<dyn maidan_search::Search> =
        Arc::new(maidan_search::SqliteSearch::new(pool.clone()));
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

    let ws = store
        .create_workspace(NewWorkspace {
            name: "states".into(),
        })
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
    let (member, token) = agent(store.as_ref(), ws.id, "agent").await;
    Room {
        base: format!("http://{addr}"),
        store,
        pool,
        channel: channel.id,
        member,
        token,
    }
}

async fn agent(store: &dyn Store, ws: WorkspaceId, handle: &str) -> (MemberId, String) {
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
            capabilities: AGENT.iter().map(|c| (*c).to_string()).collect(),
            expires_at: None,
        })
        .await
        .unwrap();
    (member.id, secret.as_str().to_string())
}

async fn thread(r: &Room, title: &str) -> String {
    let created: Value = reqwest::Client::new()
        .post(format!("{}/channels/{}/threads", r.base, r.channel.0))
        .bearer_auth(&r.token)
        .json(&json!({ "title": title }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    created["id"].as_str().unwrap().to_string()
}

async fn send(r: &Room, method: reqwest::Method, path: &str, body: Value) -> (StatusCode, Value) {
    let resp = reqwest::Client::new()
        .request(method, format!("{}{path}", r.base))
        .bearer_auth(&r.token)
        .json(&body)
        .send()
        .await
        .unwrap();
    let status = resp.status();
    (status, resp.json().await.unwrap_or(Value::Null))
}

async fn tool(r: &Room, name: &str, arguments: Value) -> Value {
    reqwest::Client::new()
        .post(format!("{}/mcp", r.base))
        .bearer_auth(&r.token)
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

/// Walk a thread to `closed` (and on to `archived`) through the transition
/// route, as a person would.
async fn end(r: &Room, thread: &str, archive: bool) {
    let mut actions = vec!["start_review", "close"];
    if archive {
        actions.push("archive");
    }
    for action in actions {
        let (status, body) = send(
            r,
            reqwest::Method::POST,
            &format!("/threads/{thread}"),
            json!({ "action": action }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{action}: {body}");
    }
}

fn problem_type(body: &Value) -> &str {
    body["type"].as_str().unwrap_or_default()
}

/// The JSON-RPC error of a tool call, or a panic naming what came back.
fn rpc_error(response: &Value) -> &Value {
    let error = &response["error"];
    assert!(error.is_object(), "expected a JSON-RPC error: {response}");
    error
}

#[tokio::test]
async fn a_renewal_or_result_on_an_expired_lease_is_claim_lost_before_any_reaper_runs() {
    let r = room().await;
    let thread = thread(&r, "leased").await;
    let (status, claimed) = send(
        &r,
        reqwest::Method::POST,
        &format!("/channels/{}/threads/claim-next", r.channel.0),
        json!({ "lease_secs": 600 }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{claimed}");
    let held = if claimed["thread"].is_object() {
        claimed["thread"].clone()
    } else {
        claimed
    };
    assert_eq!(held["id"].as_str(), Some(thread.as_str()));
    let lease = held["claim_lease_id"].as_str().unwrap().to_string();
    let thread_id = ThreadId(thread.parse().unwrap());

    // The deadline passes; nothing has reaped the claim.
    r.store
        .renew_claim(
            thread_id,
            r.member,
            maidan_types::ClaimLeaseId(lease.parse().unwrap()),
            -5,
        )
        .await
        .unwrap();
    assert_eq!(
        r.store.get_thread(thread_id).await.unwrap().assignee_id,
        Some(r.member),
        "still assigned: not reaped"
    );

    let (status, body) = send(
        &r,
        reqwest::Method::POST,
        &format!("/threads/{thread}/claim/renew"),
        json!({ "claim_lease_id": lease, "lease_secs": 600 }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(
        problem_type(&body),
        "https://maidan.world/problems/claim-lost"
    );

    let (status, body) = send(
        &r,
        reqwest::Method::PUT,
        &format!("/threads/{thread}/result"),
        json!({ "claim_lease_id": lease, "result": { "kind": "test.done" } }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(
        problem_type(&body),
        "https://maidan.world/problems/claim-lost"
    );
    assert!(r
        .store
        .get_thread_result(thread_id)
        .await
        .unwrap()
        .is_none());

    // Over MCP: its own server-defined code, not -32602 invalid params, with
    // the literal `claim_lost` kept in the message and in `data.code`.
    for (name, arguments) in [
        (
            "renew_claim",
            json!({ "thread_id": thread, "claim_lease_id": lease, "lease_secs": 600 }),
        ),
        (
            "set_thread_result",
            json!({ "thread_id": thread, "claim_lease_id": lease, "result": { "kind": "test.done" } }),
        ),
    ] {
        let response = tool(&r, name, arguments).await;
        let error = rpc_error(&response);
        assert_eq!(error["code"], json!(-32012), "{name}: {response}");
        assert_eq!(error["data"]["code"], json!("claim_lost"), "{name}");
        assert!(
            error["message"].as_str().unwrap().contains("claim_lost"),
            "{name}: {response}"
        );
    }
    assert!(
        r.store
            .get_thread(thread_id)
            .await
            .unwrap()
            .assignment_expires_at
            .is_some_and(|deadline| deadline < chrono::Utc::now()),
        "no refused renewal moved the deadline"
    );
}

#[tokio::test]
async fn a_message_to_an_archived_thread_is_409_to_a_deleted_one_410_and_to_a_closed_one_201() {
    let r = room().await;
    let post = |thread: String| {
        let r = &r;
        async move {
            send(
                r,
                reqwest::Method::POST,
                &format!("/threads/{thread}/messages"),
                json!({ "body": "a note" }),
            )
            .await
        }
    };

    // People discuss the work after it closes.
    let closed = thread(&r, "closed").await;
    end(&r, &closed, false).await;
    let (status, body) = post(closed.clone()).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");

    let archived = thread(&r, "archived").await;
    end(&r, &archived, true).await;
    let (status, body) = post(archived.clone()).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(
        problem_type(&body),
        "https://maidan.world/problems/thread-ended"
    );

    let deleted = thread(&r, "deleted").await;
    sqlx::query("UPDATE maidan_threads SET tombstoned_at = ? WHERE id = ?")
        .bind(chrono::Utc::now().to_rfc3339())
        .bind(uuid::Uuid::parse_str(&deleted).unwrap())
        .execute(&r.pool)
        .await
        .unwrap();
    let (status, body) = post(deleted.clone()).await;
    assert_eq!(status, StatusCode::GONE, "{body}");
    assert_eq!(problem_type(&body), "https://maidan.world/problems/gone");

    for (thread, code, data) in [
        (&archived, -32013, "thread_ended"),
        (&deleted, -32014, "gone"),
    ] {
        let response = tool(
            &r,
            "post_message",
            json!({ "thread_id": thread, "body": "a note" }),
        )
        .await;
        let error = rpc_error(&response);
        assert_eq!(error["code"], json!(code), "{response}");
        assert_eq!(error["data"]["code"], json!(data), "{response}");
    }
    for refused in [&archived, &deleted] {
        let id = ThreadId(refused.parse().unwrap());
        assert!(
            r.store.list_messages(id, 10).await.unwrap().is_empty(),
            "nothing was posted to {refused}"
        );
    }
}

#[tokio::test]
async fn a_footprint_or_required_skill_on_a_closed_thread_is_409_and_an_unclaimed_thread_takes_a_footprint(
) {
    let r = room().await;
    let footprint = json!({
        "repo": "example/app",
        "mode": "exclusive",
        "items": [{ "kind": "lane", "value": "bridge" }]
    });

    // Declared before anyone claims it: `claim_next` reads it to decide.
    let open = thread(&r, "unclaimed").await;
    let (status, body) = send(
        &r,
        reqwest::Method::PUT,
        &format!("/threads/{open}/footprint"),
        footprint.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = send(
        &r,
        reqwest::Method::POST,
        &format!("/threads/{open}/required-skills"),
        json!({ "skill": "rust" }),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

    let closed = thread(&r, "closed").await;
    end(&r, &closed, false).await;
    let (status, body) = send(
        &r,
        reqwest::Method::PUT,
        &format!("/threads/{closed}/footprint"),
        footprint,
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(
        problem_type(&body),
        "https://maidan.world/problems/thread-ended"
    );
    let (status, body) = send(
        &r,
        reqwest::Method::POST,
        &format!("/threads/{closed}/required-skills"),
        json!({ "skill": "rust" }),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(
        problem_type(&body),
        "https://maidan.world/problems/thread-ended"
    );

    let closed_id = ThreadId(closed.parse().unwrap());
    assert!(r
        .store
        .get_thread_footprint(closed_id)
        .await
        .unwrap()
        .is_none());
    assert!(r
        .store
        .list_thread_required_skills(closed_id)
        .await
        .unwrap()
        .is_empty());

    let response = tool(
        &r,
        "set_thread_footprint",
        json!({
            "thread_id": closed,
            "repo": "example/app",
            "mode": "exclusive",
            "items": [{ "kind": "lane", "value": "bridge" }]
        }),
    )
    .await;
    assert_eq!(rpc_error(&response)["code"], json!(-32013), "{response}");
}

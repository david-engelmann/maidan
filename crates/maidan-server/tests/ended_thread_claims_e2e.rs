//! The claim calls answer an error the caller can act on when the thread or
//! the lease is no longer theirs to work, never a success and never a bare
//! `404` for a thread that exists:
//!
//! - an explicit claim of a closed or archived thread (closed through review,
//!   closed as abandoned, cancelled) is `409 thread-ended` / `-32013`, and of
//!   a deleted one `410 gone` / `-32014`, before the park a cancel leaves;
//! - an acknowledge or a release on a lease that expired, before any reaper
//!   ran, is `409 claim-lost` / `-32012`, as a renewal is;
//! - a thread that does not exist is still `404` / `-32004`.

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

async fn claim(r: &Room, thread: &str) -> (StatusCode, Value) {
    send(
        r,
        reqwest::Method::POST,
        &format!("/threads/{thread}/assignee/claim"),
        json!({}),
    )
    .await
}

async fn assignee(r: &Room, thread: &str) -> Option<MemberId> {
    r.store
        .get_thread(ThreadId(thread.parse().unwrap()))
        .await
        .unwrap()
        .assignee_id
}

#[tokio::test]
async fn a_claim_of_an_ended_thread_is_409_thread_ended_and_of_a_deleted_one_410() {
    let r = room().await;

    let closed = thread(&r, "closed through review").await;
    end(&r, &closed, false).await;
    let archived = thread(&r, "archived").await;
    end(&r, &archived, true).await;
    // A close as abandoned leaves no park, so only the state refuses it. A
    // cancel leaves one, and the claim still answers `thread-ended`, not the
    // park's plain conflict.
    let abandoned = thread(&r, "abandoned").await;
    let (status, body) = send(
        &r,
        reqwest::Method::POST,
        &format!("/threads/{abandoned}/close-abandoned"),
        json!({ "reason": "abandoned" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let cancelled = thread(&r, "cancelled").await;
    let (status, body) = send(
        &r,
        reqwest::Method::POST,
        &format!("/threads/{cancelled}/cancel"),
        json!({ "reason": "not needed" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");

    for ended in [&closed, &archived, &abandoned, &cancelled] {
        let (status, body) = claim(&r, ended).await;
        assert_eq!(status, StatusCode::CONFLICT, "{ended}: {body}");
        assert_eq!(
            problem_type(&body),
            "https://maidan.world/problems/thread-ended",
            "{ended}: {body}"
        );
        let response = tool(&r, "claim_thread", json!({ "thread_id": ended })).await;
        let error = rpc_error(&response);
        assert_eq!(error["code"], json!(-32013), "{ended}: {response}");
        assert_eq!(error["data"]["code"], json!("thread_ended"), "{response}");
        assert_eq!(assignee(&r, ended).await, None, "{ended}: nothing claimed");
    }

    let deleted = thread(&r, "deleted").await;
    sqlx::query("UPDATE maidan_threads SET tombstoned_at = ? WHERE id = ?")
        .bind(chrono::Utc::now().to_rfc3339())
        .bind(uuid::Uuid::parse_str(&deleted).unwrap())
        .execute(&r.pool)
        .await
        .unwrap();
    let (status, body) = claim(&r, &deleted).await;
    assert_eq!(status, StatusCode::GONE, "{body}");
    assert_eq!(problem_type(&body), "https://maidan.world/problems/gone");
    let response = tool(&r, "claim_thread", json!({ "thread_id": deleted })).await;
    let error = rpc_error(&response);
    assert_eq!(error["code"], json!(-32014), "{response}");
    assert_eq!(error["data"]["code"], json!("gone"), "{response}");

    let (status, body) = claim(&r, &uuid::Uuid::now_v7().to_string()).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

    // An open thread is still claimed.
    let open = thread(&r, "open").await;
    let (status, body) = claim(&r, &open).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["claimed"], json!(true), "{body}");
    assert_eq!(assignee(&r, &open).await, Some(r.member));
}

#[tokio::test]
async fn an_acknowledge_or_release_on_a_dead_lease_is_409_claim_lost_and_a_missing_thread_404() {
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

    for route in ["acknowledge", "release"] {
        let (status, body) = send(
            &r,
            reqwest::Method::POST,
            &format!("/threads/{thread}/claim/{route}"),
            json!({ "claim_lease_id": lease }),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{route}: {body}");
        assert_eq!(
            problem_type(&body),
            "https://maidan.world/problems/claim-lost",
            "{route}: {body}"
        );
    }
    for name in ["acknowledge_claim", "release_claim"] {
        let response = tool(
            &r,
            name,
            json!({ "thread_id": thread, "claim_lease_id": lease }),
        )
        .await;
        let error = rpc_error(&response);
        assert_eq!(error["code"], json!(-32012), "{name}: {response}");
        assert_eq!(error["data"]["code"], json!("claim_lost"), "{name}");
    }
    let after = r.store.get_thread(thread_id).await.unwrap();
    assert_eq!(after.assignee_id, Some(r.member), "nothing was released");
    assert_eq!(after.work_started_at, None, "the clock did not start");

    // A thread that does not exist is still 404, over both.
    let missing = uuid::Uuid::now_v7().to_string();
    for route in ["acknowledge", "release"] {
        let (status, body) = send(
            &r,
            reqwest::Method::POST,
            &format!("/threads/{missing}/claim/{route}"),
            json!({ "claim_lease_id": lease }),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{route}: {body}");
    }
    for name in ["acknowledge_claim", "release_claim"] {
        let response = tool(
            &r,
            name,
            json!({ "thread_id": missing, "claim_lease_id": lease }),
        )
        .await;
        assert_eq!(
            rpc_error(&response)["code"],
            json!(-32004),
            "{name}: {response}"
        );
    }
}

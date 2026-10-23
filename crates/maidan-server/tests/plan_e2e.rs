//! Planner fan-out over the wire (#1421): the issue's three-child plan made
//! in one call by the planner holding the plan's claim, replayed and refused
//! as the amendments say, dispatched in dependency order, and closed when its
//! last child lands. Another workspace can neither plan it nor read it.

use maidan_store::test_support::SetResultClaiming;
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
    ChangeSettlement, ChannelId, EgressSurface, MemberId, MemberKind, NewApiToken, NewChangeRound,
    NewChannel, NewEgressTarget, NewMember, NewWorkspace, ThreadId, WorkspaceId,
};
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

const REPO: &str = "example/app";

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

const CAPS: &[&str] = &[
    capability::WORKSPACE_READ,
    capability::WORKSPACE_WRITE,
    capability::THREAD_TRANSITION,
];

struct Room {
    ws: WorkspaceId,
    channel: ChannelId,
    planner_token: String,
    worker: MemberId,
    worker_token: String,
}

async fn room(store: &dyn Store, name: &str) -> Room {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap();
    let (_, planner_token) = member_with(store, ws.id, "planner", CAPS).await;
    let (worker, worker_token) = member_with(store, ws.id, "worker", CAPS).await;
    let channel = store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "work".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    store
        .allow_egress_target(NewEgressTarget {
            workspace_id: ws.id,
            surface: EgressSurface::GithubBranch,
            selector: format!("{REPO}@main"),
        })
        .await
        .unwrap();
    Room {
        ws: ws.id,
        channel: channel.id,
        planner_token,
        worker,
        worker_token,
    }
}

/// The claimed thread out of a claim-next answer, whichever shape it has.
fn claimed(answer: &Value) -> Option<&Value> {
    let thread = if answer["thread"].is_object() {
        &answer["thread"]
    } else {
        answer
    };
    thread["id"].is_string().then_some(thread)
}

async fn claim_next(base: &str, room: &Room, token: &str) -> Option<Value> {
    let answer: Value = reqwest::Client::new()
        .post(format!(
            "{base}/channels/{}/threads/claim-next",
            room.channel.0
        ))
        .bearer_auth(token)
        .json(&json!({ "lease_secs": 600 }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap_or(Value::Null);
    claimed(&answer).cloned()
}

async fn plan(base: &str, token: &str, parent: &str, body: &Value) -> (StatusCode, Value) {
    let response = reqwest::Client::new()
        .post(format!("{base}/threads/{parent}/plan"))
        .bearer_auth(token)
        .json(body)
        .send()
        .await
        .unwrap();
    let status = response.status();
    (status, response.json().await.unwrap_or(Value::Null))
}

async fn get_thread(base: &str, token: &str, id: &str) -> (StatusCode, Value) {
    let response = reqwest::Client::new()
        .get(format!("{base}/threads/{id}"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    let status = response.status();
    (status, response.json().await.unwrap_or(Value::Null))
}

fn child(key: &str, lane: &str, deps: &[&str]) -> Value {
    json!({
        "key": key,
        "title": format!("do {key}"),
        "attributes": { "kind": "change", "engine": "pi", "size": "S", "acceptance": format!("{key} works") },
        "test": format!("cargo test -p {key}"),
        "footprint": {
            "repo": REPO,
            "mode": "exclusive",
            "items": [{ "kind": "lane", "value": lane }]
        },
        "depends_on": deps,
    })
}

fn key_id(children: &Value, key: &str) -> String {
    children["children"]
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["key"] == key)
        .and_then(|c| c["thread_id"].as_str())
        .unwrap()
        .to_string()
}

/// Land a child as the PR sweep would: a result, a round, a merge, a settle.
async fn land(store: &dyn Store, room: &Room, id: &str, pr: i64) {
    let thread = ThreadId(id.parse().unwrap());
    store
        .set_thread_result_claiming(thread, room.worker, &json!({"summary": "done"}))
        .await
        .unwrap();
    store
        .record_change_round(NewChangeRound {
            workspace_id: room.ws,
            thread_id: thread,
            repo: REPO.into(),
            pull_number: pr,
            commit_sha: format!("{pr:040x}"),
            base_sha: "a".repeat(40),
            base_behind: None,
            diff_lines: 1,
            paths: Vec::new(),
            base_branch: None,
            rebase: false,
            previous_head: None,
            linear_issue: None,
        })
        .await
        .unwrap();
    let at = chrono::Utc::now();
    store
        .close_change_pull(room.ws, REPO, pr, Some(at), Some("c0ffee"), at)
        .await
        .unwrap();
    assert!(matches!(
        store.settle_change_pull(room.ws, REPO, pr).await.unwrap(),
        ChangeSettlement::Landed(_)
    ));
}

#[tokio::test]
async fn the_issue_plan_fans_out_dispatches_in_order_and_lands() {
    let (base, store) = spawn().await;
    let room = room(store.as_ref(), "plan-e2e").await;

    let response = reqwest::Client::new()
        .post(format!("{base}/channels/{}/threads", room.channel.0))
        .bearer_auth(&room.planner_token)
        .json(&json!({ "title": "ship bookings", "attributes": { "kind": "plan" } }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CREATED);
    let parent = response.json::<Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let held = claim_next(&base, &room, &room.planner_token)
        .await
        .expect("the plan is the only work");
    assert_eq!(held["id"], parent.as_str());
    let lease = held["claim_lease_id"].as_str().unwrap().to_string();

    let body = json!({
        "lease_id": lease,
        "children": [child("a", "schema", &[]), child("b", "api", &["a"]), child("c", "docs", &[])],
    });
    // Only the claim holder plans.
    let (status, problem) = plan(&base, &room.worker_token, &parent, &body).await;
    assert_eq!(status, StatusCode::CONFLICT, "{problem}");
    // Another workspace never reaches the plan at all.
    let other = room_token(store.as_ref()).await;
    let (status, _) = plan(&base, &other, &parent, &body).await;
    assert!(
        matches!(status, StatusCode::FORBIDDEN | StatusCode::NOT_FOUND),
        "another tenant got {status}"
    );

    let (status, made) = plan(&base, &room.planner_token, &parent, &body).await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    assert_eq!(made["children"].as_array().unwrap().len(), 3);
    let (a, b, c) = (key_id(&made, "a"), key_id(&made, "b"), key_id(&made, "c"));

    let (status, again) = plan(&base, &room.planner_token, &parent, &body).await;
    assert_eq!(status, StatusCode::OK, "the same claim and keys replay");
    assert_eq!(again, made);
    let mut fewer = body.clone();
    fewer["children"] = json!([child("a", "schema", &[])]);
    let (status, problem) = plan(&base, &room.planner_token, &parent, &fewer).await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(problem["type"], "https://maidan.dev/problems/plan-conflict");
    assert_eq!(problem["children"], made["children"]);

    let (_, read) = get_thread(&base, &room.planner_token, &parent).await;
    assert_eq!(
        read["plan_state"],
        json!({"total": 3, "closed": 0, "open": 3, "blocked": 0})
    );
    let (status, _) = get_thread(&base, &other, &parent).await;
    assert!(matches!(
        status,
        StatusCode::FORBIDDEN | StatusCode::NOT_FOUND
    ));

    // a and c go out; b waits for a.
    let mut first: Vec<String> = Vec::new();
    while let Some(t) = claim_next(&base, &room, &room.worker_token).await {
        first.push(t["id"].as_str().unwrap().to_string());
    }
    first.sort();
    let mut expected = vec![a.clone(), c.clone()];
    expected.sort();
    assert_eq!(first, expected);

    land(store.as_ref(), &room, &a, 1).await;
    let next = claim_next(&base, &room, &room.worker_token)
        .await
        .expect("b is free once a lands");
    assert_eq!(next["id"], b.as_str());

    land(store.as_ref(), &room, &c, 2).await;
    let (_, read) = get_thread(&base, &room.planner_token, &parent).await;
    assert_eq!(read["state"], "open", "b has not landed");
    land(store.as_ref(), &room, &b, 3).await;
    let (_, read) = get_thread(&base, &room.planner_token, &parent).await;
    assert_eq!(read["state"], "closed");
    assert_eq!(
        read["plan_state"],
        json!({"total": 3, "closed": 3, "open": 0, "blocked": 0})
    );
}

/// A replay answers with the stored plan's hold: the first submission
/// needs a threat model and is held; the retry drops the flag, and the
/// answer still says held, because the stored children still wait.
#[tokio::test]
async fn a_replay_reports_the_stored_plans_hold() {
    let (base, store) = spawn().await;
    let room = room(store.as_ref(), "plan-e2e-held-replay").await;
    let response = reqwest::Client::new()
        .post(format!("{base}/channels/{}/threads", room.channel.0))
        .bearer_auth(&room.planner_token)
        .json(&json!({ "title": "plan", "attributes": { "kind": "plan" } }))
        .send()
        .await
        .unwrap();
    let parent = response.json::<Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let claim = claim_next(&base, &room, &room.planner_token).await.unwrap();
    let lease = claim["claim_lease_id"].as_str().unwrap().to_string();

    let mut sensitive = child("a", "schema", &[]);
    sensitive["attributes"]["engine"] = json!("session");
    sensitive["attributes"]["threat_model"] = json!(true);
    let body = json!({
        "lease_id": lease,
        "children": [sensitive.clone(), child("b", "api", &["a"])],
    });
    let (status, made) = plan(&base, &room.planner_token, &parent, &body).await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    assert_eq!(made["held"], true, "{made}");

    // The retry drops the threat model — the child goes to a session
    // either way, so the lint still passes — and still replays.
    sensitive["attributes"]
        .as_object_mut()
        .unwrap()
        .remove("threat_model");
    let retry = json!({
        "lease_id": lease,
        "children": [sensitive, child("b", "api", &["a"])],
    });
    let (status, again) = plan(&base, &room.planner_token, &parent, &retry).await;
    assert_eq!(status, StatusCode::OK, "{again}");
    assert_eq!(again["children"], made["children"]);
    assert_eq!(
        again["held"], true,
        "the stored plan is still held: {again}"
    );
}

/// A token in a workspace of its own.
async fn room_token(store: &dyn Store) -> String {
    let ws = store
        .create_workspace(NewWorkspace {
            name: "elsewhere".into(),
        })
        .await
        .unwrap();
    member_with(store, ws.id, "outsider", CAPS).await.1
}

#[tokio::test]
async fn a_malformed_plan_is_a_400_naming_the_child() {
    let (base, store) = spawn().await;
    let room = room(store.as_ref(), "plan-e2e-bad").await;
    let response = reqwest::Client::new()
        .post(format!("{base}/channels/{}/threads", room.channel.0))
        .bearer_auth(&room.planner_token)
        .json(&json!({ "title": "plan", "attributes": { "kind": "plan" } }))
        .send()
        .await
        .unwrap();
    let parent = response.json::<Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let held = claim_next(&base, &room, &room.planner_token).await.unwrap();
    let lease = held["claim_lease_id"].as_str().unwrap();
    let (status, problem) = plan(
        &base,
        &room.planner_token,
        &parent,
        &json!({
            "lease_id": lease,
            "children": [child("a", "x", &["b"]), child("b", "y", &["a"])],
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        problem["detail"]
            .as_str()
            .unwrap_or("")
            .contains("a -> b -> a")
            || problem["detail"]
                .as_str()
                .unwrap_or("")
                .contains("b -> a -> b"),
        "{problem}"
    );
    let (_, read) = get_thread(&base, &room.planner_token, &parent).await;
    assert!(
        read.get("plan_state").is_none(),
        "nothing was created: {read}"
    );
}

async fn amend(base: &str, token: &str, parent: &str, body: &Value) -> (StatusCode, Value) {
    let response = reqwest::Client::new()
        .post(format!("{base}/threads/{parent}/plan/amend"))
        .bearer_auth(token)
        .json(body)
        .send()
        .await
        .unwrap();
    let status = response.status();
    (status, response.json().await.unwrap_or(Value::Null))
}

async fn mcp_call(base: &str, token: &str, tool: &str, args: Value) -> Value {
    reqwest::Client::new()
        .post(format!("{base}/mcp"))
        .bearer_auth(token)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": tool, "arguments": args }
        }))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap()
}

/// The planner replaces a child over REST and cancels one over MCP; another
/// workspace reaches neither, and a malformed amendment is a 400 that
/// changes nothing.
#[tokio::test]
async fn a_planner_amends_its_plan_over_rest_and_mcp_and_another_workspace_cannot() {
    let (base, store) = spawn().await;
    let room = room(store.as_ref(), "amend-e2e").await;
    let response = reqwest::Client::new()
        .post(format!("{base}/channels/{}/threads", room.channel.0))
        .bearer_auth(&room.planner_token)
        .json(&json!({ "title": "ship bookings", "attributes": { "kind": "plan" } }))
        .send()
        .await
        .unwrap();
    let parent = response.json::<Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let held = claim_next(&base, &room, &room.planner_token).await.unwrap();
    let lease = held["claim_lease_id"].as_str().unwrap().to_string();
    let (status, made) = plan(
        &base,
        &room.planner_token,
        &parent,
        &json!({
            "lease_id": lease,
            "children": [child("a", "schema", &[]), child("b", "api", &["a"]), child("c", "docs", &["b"])],
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{made}");
    let old_b = key_id(&made, "b");

    let replace = json!({
        "lease_id": lease,
        "cancel": ["b"],
        "add": [child("b", "api-v2", &["a"])],
        "reason": "b needs the new api shape",
    });
    let other = room_token(store.as_ref()).await;
    let (status, _) = amend(&base, &other, &parent, &replace).await;
    assert!(
        matches!(status, StatusCode::FORBIDDEN | StatusCode::NOT_FOUND),
        "another tenant got {status}"
    );
    let (status, problem) = amend(
        &base,
        &room.planner_token,
        &parent,
        &json!({ "lease_id": lease, "cancel": ["ghost"] }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{problem}");

    let (status, amended) = amend(&base, &room.planner_token, &parent, &replace).await;
    assert_eq!(status, StatusCode::OK, "{amended}");
    assert_eq!(amended["amendments"], 1);
    let new_b = amended["added"][0]["thread_id"]
        .as_str()
        .unwrap()
        .to_string();
    assert_ne!(new_b, old_b);
    let (_, old) = get_thread(&base, &room.planner_token, &old_b).await;
    assert_eq!(old["state"], "closed");

    let answer = mcp_call(
        &base,
        &room.planner_token,
        "amend_plan",
        json!({ "thread_id": parent, "lease_id": lease, "cancel": ["c"] }),
    )
    .await;
    let text = answer["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("no tool result: {answer}"));
    let cancelled: Value = serde_json::from_str(text).unwrap();
    assert_eq!(cancelled["amendments"], 2);
    assert_eq!(cancelled["cancelled"][0]["key"], "c");
    let other_answer = mcp_call(
        &base,
        &other,
        "amend_plan",
        json!({ "thread_id": parent, "lease_id": lease, "cancel": ["a"] }),
    )
    .await;
    assert!(
        other_answer["error"].is_object() || other_answer["result"]["isError"] == true,
        "{other_answer}"
    );
    let (_, a) = get_thread(&base, &room.planner_token, &key_id(&made, "a")).await;
    assert_eq!(a["state"], "open");
}

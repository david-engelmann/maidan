//! Task attributes over the wire: set at intake on REST and MCP create,
//! read on the thread, replaced by the PUT and the MCP tool, respected by
//! an engine-filtered claim, invisible and unchangeable to another tenant,
//! and joined onto the swarm metrics with their filters and engine split.

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
    ChannelId, MemberId, MemberKind, NewApiToken, NewChangeRound, NewChannel, NewMember,
    NewWorkspace, WorkspaceId,
};
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

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
    token: String,
}

async fn room(base_store: &dyn Store, name: &str) -> Room {
    let ws = base_store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap();
    let (_member, token) = member_with(base_store, ws.id, "agent", CAPS).await;
    let channel = base_store
        .create_channel(NewChannel {
            workspace_id: ws.id,
            name: "work".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    Room {
        ws: ws.id,
        channel: channel.id,
        token,
    }
}

async fn create_thread(base: &str, room: &Room, body: Value) -> (StatusCode, Value) {
    let response = reqwest::Client::new()
        .post(format!("{base}/channels/{}/threads", room.channel.0))
        .bearer_auth(&room.token)
        .json(&body)
        .send()
        .await
        .unwrap();
    let status = response.status();
    (status, response.json().await.unwrap_or(Value::Null))
}

async fn get_thread(base: &str, token: &str, thread_id: &str) -> (StatusCode, Value) {
    let response = reqwest::Client::new()
        .get(format!("{base}/threads/{thread_id}"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    let status = response.status();
    (status, response.json().await.unwrap_or(Value::Null))
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

/// The JSON a successful tool call returned.
fn tool_result(response: &Value) -> Value {
    let text = response["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("not a tool result: {response}"));
    serde_json::from_str(text).unwrap()
}

#[tokio::test]
async fn attributes_ride_create_read_and_replace_on_rest_and_mcp() {
    let (base, store) = spawn().await;
    let room = room(store.as_ref(), "attrs").await;

    // Set at intake, over REST.
    let (status, created) = create_thread(
        &base,
        &room,
        json!({
            "title": "review the gateway",
            "attributes": { "kind": "review", "engine": "pi", "threat_model": true }
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(
        created["attributes"],
        json!({ "kind": "review", "engine": "pi", "threat_model": true })
    );
    let thread_id = created["id"].as_str().unwrap().to_string();

    // Read back on the thread.
    let (status, read) = get_thread(&base, &room.token, &thread_id).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(read["attributes"]["kind"], "review");

    // Replaced whole by the PUT.
    let response = reqwest::Client::new()
        .put(format!("{base}/threads/{thread_id}/attributes"))
        .bearer_auth(&room.token)
        .json(&json!({ "kind": "plan", "engine": "session", "threat_model": false }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let replaced: Value = response.json().await.unwrap();
    assert_eq!(
        replaced,
        json!({ "kind": "plan", "engine": "session", "threat_model": false })
    );
    let (_status, read) = get_thread(&base, &room.token, &thread_id).await;
    assert_eq!(read["attributes"]["kind"], "plan");

    // A thread created without attributes reads as the defaults, and the
    // MCP create + tool pair carries them too.
    let (status, plain) = create_thread(&base, &room, json!({ "title": "plain" })).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(
        plain["attributes"],
        json!({ "kind": "change", "threat_model": false }),
        "an undescribed thread reads as a change with no engine"
    );

    let made = tool_result(
        &tool(
            &base,
            &room.token,
            "create_thread",
            json!({
                "channel_id": room.channel.0,
                "title": "from mcp",
                "attributes": { "kind": "change", "engine": "human" }
            }),
        )
        .await,
    );
    assert_eq!(made["attributes"]["engine"], "human");
    let mcp_thread = made["id"].as_str().unwrap().to_string();
    let set = tool_result(
        &tool(
            &base,
            &room.token,
            "set_thread_attributes",
            json!({
                "thread_id": mcp_thread,
                "kind": "review",
                "engine": "pi",
                "threat_model": true
            }),
        )
        .await,
    );
    assert_eq!(set["kind"], "review");
    let (_status, read) = get_thread(&base, &room.token, &mcp_thread).await;
    assert_eq!(
        read["attributes"],
        json!({ "kind": "review", "engine": "pi", "threat_model": true })
    );
}

#[tokio::test]
async fn a_bad_attribute_value_creates_no_thread() {
    let (base, store) = spawn().await;
    let room = room(store.as_ref(), "bad-attrs").await;
    let (status, _body) = create_thread(
        &base,
        &room,
        json!({
            "title": "bogus kind",
            "attributes": { "kind": "bogus" }
        }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let threads = store.list_threads(room.channel).await.unwrap();
    assert!(
        threads.is_empty(),
        "a refused create must leave no thread behind"
    );

    // The PUT refuses a bad value the same way.
    let (status, ok) = create_thread(&base, &room, json!({ "title": "fine" })).await;
    assert_eq!(status, StatusCode::CREATED);
    let thread_id = ok["id"].as_str().unwrap();
    let response = reqwest::Client::new()
        .put(format!("{base}/threads/{thread_id}/attributes"))
        .bearer_auth(&room.token)
        .json(&json!({ "kind": "change", "engine": "robot" }))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_wrapped_or_unknown_attribute_body_is_refused_and_changes_nothing() {
    let (base, store) = spawn().await;
    let room = room(store.as_ref(), "strict-attrs").await;
    let set = json!({
        "kind": "change",
        "engine": "pi",
        "acceptance": "the gate passes",
        "linear_issue": "ENG-6023"
    });
    let (status, created) = create_thread(
        &base,
        &room,
        json!({ "title": "keep me", "attributes": set }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    let thread_id = created["id"].as_str().unwrap().to_string();

    for body in [
        json!({ "attributes": { "kind": "change", "engine": "session" } }),
        json!({ "kind": "change", "twin_of": "ENG-6001" }),
    ] {
        let response = reqwest::Client::new()
            .put(format!("{base}/threads/{thread_id}/attributes"))
            .bearer_auth(&room.token)
            .json(&body)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{body}");
        let (_, read) = get_thread(&base, &room.token, &thread_id).await;
        let mut kept = set.clone();
        kept["threat_model"] = json!(false);
        assert_eq!(
            read["attributes"], kept,
            "a refused body must change nothing"
        );
    }

    // Intake refuses an unknown key too, and makes no thread.
    let (status, _) = create_thread(
        &base,
        &room,
        json!({ "title": "typo", "attributes": { "kind": "change", "engnie": "pi" } }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(store.list_threads(room.channel).await.unwrap().len(), 1);
}

#[tokio::test]
async fn an_engine_filtered_claim_skips_other_engines() {
    let (base, store) = spawn().await;
    let room = room(store.as_ref(), "claim-engines").await;
    for (title, engine) in [
        ("session work", Some("session")),
        ("pi work", Some("pi")),
        ("unrouted work", None),
    ] {
        let mut attributes = json!({ "kind": "change" });
        if let Some(engine) = engine {
            attributes["engine"] = json!(engine);
        }
        let (status, _created) = create_thread(
            &base,
            &room,
            json!({ "title": title, "attributes": attributes }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
    }

    let http = reqwest::Client::new();
    let claim = |engine: &str| {
        let http = http.clone();
        let url = format!("{base}/channels/{}/threads/claim-next", room.channel.0);
        let token = room.token.clone();
        let engine = engine.to_string();
        async move {
            http.post(url)
                .bearer_auth(token)
                .json(&json!({ "lease_secs": 600, "engine": engine }))
                .send()
                .await
                .unwrap()
                .json::<Value>()
                .await
                .unwrap()
        }
    };
    let pi_claim = claim("pi").await;
    assert_eq!(
        pi_claim["title"], "pi work",
        "the pi claim takes the pi task, not the older session or unrouted work: {pi_claim}"
    );
    let second = claim("pi").await;
    assert!(
        second.is_null(),
        "no second pi task exists, so the claim is empty: {second}"
    );
    let human = claim("human").await;
    assert!(
        human.is_null(),
        "human work was never posted, so a human claim is empty: {human}"
    );
}

#[tokio::test]
async fn another_workspace_cannot_read_or_replace_attributes() {
    let (base, store) = spawn().await;
    let victim = room(store.as_ref(), "victim").await;
    let attacker = room(store.as_ref(), "attacker").await;
    let (status, created) = create_thread(
        &base,
        &victim,
        json!({
            "title": "victim task",
            "attributes": { "kind": "review", "engine": "pi", "threat_model": true }
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let thread_id = created["id"].as_str().unwrap().to_string();

    let (status, _body) = get_thread(&base, &attacker.token, &thread_id).await;
    assert!(
        status == StatusCode::FORBIDDEN || status == StatusCode::NOT_FOUND,
        "another tenant's read must refuse, got {status}"
    );
    let response = reqwest::Client::new()
        .put(format!("{base}/threads/{thread_id}/attributes"))
        .bearer_auth(&attacker.token)
        .json(&json!({ "kind": "plan", "engine": "human" }))
        .send()
        .await
        .unwrap();
    let status = response.status();
    assert!(
        status == StatusCode::FORBIDDEN || status == StatusCode::NOT_FOUND,
        "another tenant's replace must refuse, got {status}"
    );

    let (status, read) = get_thread(&base, &victim.token, &thread_id).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        read["attributes"],
        json!({ "kind": "review", "engine": "pi", "threat_model": true }),
        "the victim's attributes are untouched"
    );
}

#[tokio::test]
async fn swarm_metrics_carry_attributes_filters_and_the_engine_split() {
    let (base, store) = spawn().await;
    let room = room(store.as_ref(), "metrics").await;
    let mut ids = Vec::new();
    for (n, attributes) in [
        (1, json!({ "kind": "review", "engine": "pi" })),
        (2, json!({ "kind": "change" })),
    ] {
        let (status, created) = create_thread(
            &base,
            &room,
            json!({ "title": format!("task {n}"), "attributes": attributes }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
        let thread_id = maidan_types::ThreadId(created["id"].as_str().unwrap().parse().unwrap());
        store
            .record_change_round(NewChangeRound {
                workspace_id: room.ws,
                thread_id,
                repo: "example/app".into(),
                pull_number: n,
                commit_sha: format!("c{n}"),
                base_sha: "a".repeat(40),
                base_behind: None,
                diff_lines: 10,
                paths: Vec::new(),
                base_branch: None,
                rebase: false,
                previous_head: None,
                linear_issue: None,
            })
            .await
            .unwrap();
        ids.push(thread_id);
    }

    let http = reqwest::Client::new();
    let metrics = |query: &str| {
        let http = http.clone();
        let url = format!("{base}/workspaces/{}/swarm-metrics{query}", room.ws.0);
        let token = room.token.clone();
        async move {
            let response = http.get(url).bearer_auth(token).send().await.unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            response.json::<Value>().await.unwrap()
        }
    };
    let all = metrics("").await;
    assert_eq!(all["tasks"].as_array().unwrap().len(), 2);
    let review_task = all["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|task| task["pull_number"] == 1)
        .expect("the review task is listed");
    assert_eq!(review_task["attributes"]["kind"], "review");
    let rollup = &all["repos"][0];
    let engines: Vec<&Value> = rollup["by_engine"].as_array().unwrap().iter().collect();
    assert_eq!(engines.len(), 2);
    assert_eq!(engines[0]["engine"], "pi");
    assert_eq!(engines[0]["tasks"], 1);
    assert!(engines[1]["engine"].is_null());
    assert_eq!(engines[1]["tasks"], 1);

    let pi_only = metrics("?engine=pi").await;
    assert_eq!(pi_only["tasks"].as_array().unwrap().len(), 1);
    let reviews = metrics("?kind=review").await;
    assert_eq!(reviews["tasks"].as_array().unwrap().len(), 1);
    assert_eq!(reviews["tasks"][0]["pull_number"], 1);
}

#[tokio::test]
async fn a_producer_sets_priority_size_and_acceptance_at_intake() {
    let (base, store) = spawn().await;
    let room = room(store.as_ref(), "intake").await;
    // A producer creates tasks; it holds no thread:transition.
    let (_, producer) = member_with(
        store.as_ref(),
        room.ws,
        "producer",
        &[
            maidan_auth::capability::WORKSPACE_READ,
            maidan_auth::capability::WORKSPACE_WRITE,
        ],
    )
    .await;
    let as_producer = Room {
        ws: room.ws,
        channel: room.channel,
        token: producer,
    };
    let (status, _older) = create_thread(&base, &as_producer, json!({ "title": "older" })).await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, urgent) = create_thread(
        &base,
        &as_producer,
        json!({
            "title": "urgent",
            "priority": 100,
            "attributes": {
                "kind": "change",
                "engine": "pi",
                "size": "S",
                "acceptance": "The retry test passes."
            }
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{urgent}");
    assert_eq!(urgent["attributes"]["size"], "S");
    assert_eq!(urgent["attributes"]["acceptance"], "The retry test passes.");

    let claimed = tool_result(
        &tool(
            &base,
            &room.token,
            "claim_next_thread",
            json!({ "channel_id": room.channel.0 }),
        )
        .await,
    );
    let id = claimed["thread"]["id"]
        .as_str()
        .or_else(|| claimed["id"].as_str())
        .unwrap_or_else(|| panic!("nothing claimed: {claimed}"));
    assert_eq!(id, urgent["id"].as_str().unwrap(), "priority beats age");

    let (status, _) = create_thread(
        &base,
        &as_producer,
        json!({ "title": "bad", "attributes": { "kind": "change", "size": "XL" } }),
    )
    .await;
    assert!(status.is_client_error(), "an unknown size is refused");
}

async fn default_engine(
    base: &str,
    token: &str,
    ws: WorkspaceId,
    put: Option<Value>,
) -> (StatusCode, Value) {
    let url = format!("{base}/workspaces/{}/default-engine", ws.0);
    let client = reqwest::Client::new();
    let request = match put {
        Some(body) => client.put(url).json(&body),
        None => client.get(url),
    };
    let response = request.bearer_auth(token).send().await.unwrap();
    let status = response.status();
    (status, response.json().await.unwrap_or(Value::Null))
}

#[tokio::test]
async fn a_default_engine_routes_an_untagged_thread_to_its_engine_claimer() {
    let (base, store) = spawn().await;
    let room = room(store.as_ref(), "defaults").await;
    let (_admin, admin) = member_with(
        store.as_ref(),
        room.ws,
        "admin",
        &[capability::TOKEN_ADMIN, capability::WORKSPACE_READ],
    )
    .await;

    let (status, unset) = default_engine(&base, &room.token, room.ws, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(unset, json!({ "engine": null }));
    // Routing every untagged thread is an operator's call.
    let (status, _) =
        default_engine(&base, &room.token, room.ws, Some(json!({ "engine": "pi" }))).await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    let (status, set) =
        default_engine(&base, &admin, room.ws, Some(json!({ "engine": "pi" }))).await;
    assert_eq!(status, StatusCode::OK, "{set}");
    assert_eq!(set, json!({ "engine": "pi" }));
    let audit = store
        .list_audit_for_workspace(room.ws, 50)
        .await
        .unwrap()
        .into_iter()
        .find(|row| row.action == "workspace.default_engine.set")
        .expect("the change is audited");
    assert_eq!(audit.metadata, json!({ "engine": "pi" }));

    let (status, created) = create_thread(&base, &room, json!({ "title": "untagged" })).await;
    assert_eq!(status, StatusCode::CREATED, "{created}");
    assert_eq!(created["attributes"]["engine"], "pi", "{created}");
    let thread_id = created["id"].as_str().unwrap().to_string();

    let claim = |engine: &'static str| {
        let url = format!("{base}/channels/{}/threads/claim-next", room.channel.0);
        let token = room.token.clone();
        async move {
            reqwest::Client::new()
                .post(url)
                .bearer_auth(token)
                .json(&json!({ "lease_secs": 600, "engine": engine }))
                .send()
                .await
                .unwrap()
                .json::<Value>()
                .await
                .unwrap()
        }
    };
    let session = claim("session").await;
    assert!(
        session.is_null(),
        "a session claimer must not take pi work: {session}"
    );
    let pi = claim("pi").await;
    assert_eq!(pi["id"], thread_id.as_str(), "{pi}");
}

#[tokio::test]
async fn another_workspace_cannot_read_or_set_the_default_engine() {
    let (base, store) = spawn().await;
    let victim = room(store.as_ref(), "victim").await;
    let attacker = room(store.as_ref(), "attacker").await;
    let (_m, victim_admin) = member_with(
        store.as_ref(),
        victim.ws,
        "admin",
        &[capability::TOKEN_ADMIN, capability::WORKSPACE_READ],
    )
    .await;
    let (_m, attacker_admin) = member_with(
        store.as_ref(),
        attacker.ws,
        "admin",
        &[capability::TOKEN_ADMIN, capability::WORKSPACE_READ],
    )
    .await;

    let (status, _) = default_engine(
        &base,
        &victim_admin,
        victim.ws,
        Some(json!({ "engine": "human" })),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let (status, attacker_own) = default_engine(&base, &attacker_admin, attacker.ws, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        attacker_own,
        json!({ "engine": null }),
        "the victim's default does not reach another workspace"
    );

    for put in [None, Some(json!({ "engine": "pi" }))] {
        let (status, body) = default_engine(&base, &attacker_admin, victim.ws, put).await;
        assert!(
            status == StatusCode::FORBIDDEN || status == StatusCode::NOT_FOUND,
            "another tenant must be refused, got {status}: {body}"
        );
    }
    let (_, read) = default_engine(&base, &victim_admin, victim.ws, None).await;
    assert_eq!(
        read,
        json!({ "engine": "human" }),
        "the victim's default holds"
    );
}

/// A worker sees what it is asked to do and how it is checked where it
/// reads its work: the claim answer (REST and MCP) and the MCP thread
/// context pack carry the task's attributes, as `GET /threads/{id}` does.
#[tokio::test]
async fn the_claim_answer_and_the_context_pack_carry_the_task_attributes() {
    let (base, store) = spawn().await;
    let room = room(store.as_ref(), "attrs-pack").await;
    let attributes = json!({
        "kind": "change",
        "engine": "pi",
        "threat_model": false,
        "size": "M",
        "acceptance": "The bridge retries twice; its test proves it.",
        "linear_issue": "ENG-123"
    });
    // Each thread names its own issue: two open change threads for one
    // issue are a likely duplicate, and claim_next withholds the second.
    let mut second = attributes.clone();
    second["linear_issue"] = json!("ENG-124");
    for (title, attrs) in [("first", &attributes), ("second", &second)] {
        let (status, made) =
            create_thread(&base, &room, json!({ "title": title, "attributes": attrs })).await;
        assert_eq!(status, StatusCode::CREATED, "{made}");
    }

    let over_mcp = tool_result(
        &tool(
            &base,
            &room.token,
            "claim_next_thread",
            json!({ "channel_id": room.channel.0, "engine": "pi" }),
        )
        .await,
    );
    let claimed = [&attributes, &second];
    assert!(claimed.contains(&&over_mcp["attributes"]), "{over_mcp}");
    let over_rest: Value = reqwest::Client::new()
        .post(format!(
            "{base}/channels/{}/threads/claim-next",
            room.channel.0
        ))
        .bearer_auth(&room.token)
        .json(&json!({ "lease_secs": 600, "engine": "pi" }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(claimed.contains(&&over_rest["attributes"]), "{over_rest}");
    assert_ne!(over_rest["id"], over_mcp["id"]);

    let pack = tool_result(
        &tool(
            &base,
            &room.token,
            "get_thread_context",
            json!({ "thread_id": over_mcp["id"] }),
        )
        .await,
    );
    assert_eq!(
        pack["thread"]["attributes"], over_mcp["attributes"],
        "{pack}"
    );
}

/// A lane other than batch or flex is refused on every REST and MCP way in,
/// at intake, on replace and on a claim, and changes nothing.
#[tokio::test]
async fn an_unknown_lane_is_refused_on_rest_and_mcp() {
    let (base, store) = spawn().await;
    let room = room(store.as_ref(), "bad-lane").await;
    let bad = json!({ "kind": "change", "lane": "standard" });

    let (status, _) = create_thread(
        &base,
        &room,
        json!({ "title": "bad lane", "attributes": bad }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let refused = tool(
        &base,
        &room.token,
        "create_thread",
        json!({ "channel_id": room.channel.0, "title": "bad lane", "attributes": bad }),
    )
    .await;
    assert_eq!(refused["error"]["code"], -32602, "{refused}");
    assert!(
        store.list_threads(room.channel).await.unwrap().is_empty(),
        "a refused lane makes no thread"
    );

    let (status, ok) = create_thread(
        &base,
        &room,
        json!({ "title": "batch work", "attributes": { "kind": "change", "lane": "batch" } }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{ok}");
    let thread_id = ok["id"].as_str().unwrap().to_string();
    let http = reqwest::Client::new();
    let response = http
        .put(format!("{base}/threads/{thread_id}/attributes"))
        .bearer_auth(&room.token)
        .json(&bad)
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let refused = tool(
        &base,
        &room.token,
        "set_thread_attributes",
        json!({ "thread_id": thread_id, "kind": "change", "lane": "standard" }),
    )
    .await;
    assert_eq!(refused["error"]["code"], -32602, "{refused}");
    let (_, read) = get_thread(&base, &room.token, &thread_id).await;
    assert_eq!(
        read["attributes"]["lane"], "batch",
        "a refused lane changes nothing"
    );

    let response = http
        .post(format!(
            "{base}/channels/{}/threads/claim-next",
            room.channel.0
        ))
        .bearer_auth(&room.token)
        .json(&json!({ "lease_secs": 600, "lane": "standard" }))
        .send()
        .await
        .unwrap();
    assert!(
        response.status().is_client_error(),
        "a claim naming an unknown lane is refused: {}",
        response.status()
    );
    let refused = tool(
        &base,
        &room.token,
        "claim_next_thread",
        json!({ "channel_id": room.channel.0, "lane": "standard" }),
    )
    .await;
    assert_eq!(refused["error"]["code"], -32602, "{refused}");
    let (_, read) = get_thread(&base, &room.token, &thread_id).await;
    assert!(
        read["assignee_id"].is_null(),
        "a refused claim takes nothing: {read}"
    );
}

/// A claim naming a lane takes that lane's thread before older unlaned work
/// of equal priority (lane breaks the tie), and a claim with no lane then
/// still takes what is left.
#[tokio::test]
async fn a_lane_claim_takes_its_lane_first_over_rest() {
    let (base, store) = spawn().await;
    let room = room(store.as_ref(), "claim-lanes").await;
    for (title, lane) in [("plain work", None), ("flex work", Some("flex"))] {
        let mut attributes = json!({ "kind": "change" });
        if let Some(lane) = lane {
            attributes["lane"] = json!(lane);
        }
        let (status, _) = create_thread(
            &base,
            &room,
            json!({ "title": title, "attributes": attributes }),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);
    }
    let http = reqwest::Client::new();
    let url = format!("{base}/channels/{}/threads/claim-next", room.channel.0);
    let claim = |body: Value| {
        let (http, url, token) = (http.clone(), url.clone(), room.token.clone());
        async move {
            http.post(url)
                .bearer_auth(token)
                .json(&body)
                .send()
                .await
                .unwrap()
                .json::<Value>()
                .await
                .unwrap()
        }
    };
    let flex = claim(json!({ "lease_secs": 600, "lane": "flex" })).await;
    assert_eq!(flex["title"], "flex work", "{flex}");
    let rest = claim(json!({ "lease_secs": 600 })).await;
    assert_eq!(rest["title"], "plain work", "{rest}");
}

async fn review_children(
    base: &str,
    token: &str,
    ws: WorkspaceId,
    put: Option<Value>,
) -> (StatusCode, Value) {
    let url = format!("{base}/workspaces/{}/review-children", ws.0);
    let client = reqwest::Client::new();
    let request = match put {
        Some(body) => client.put(url).json(&body),
        None => client.get(url),
    };
    let response = request.bearer_auth(token).send().await.unwrap();
    let status = response.status();
    (status, response.json().await.unwrap_or(Value::Null))
}

#[tokio::test]
async fn a_workspace_admin_turns_review_children_off_and_the_change_is_audited() {
    let (base, store) = spawn().await;
    let room = room(store.as_ref(), "review-children").await;
    let (_admin, admin) = member_with(
        store.as_ref(),
        room.ws,
        "admin",
        &[capability::TOKEN_ADMIN, capability::WORKSPACE_READ],
    )
    .await;

    let (status, unset) = review_children(&base, &room.token, room.ws, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(unset, json!({ "review_children": "spawn" }));
    let off = json!({ "review_children": "off" });
    let (status, _) = review_children(&base, &room.token, room.ws, Some(off.clone())).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "it is an operator's call");
    let (status, set) = review_children(&base, &admin, room.ws, Some(off.clone())).await;
    assert_eq!(status, StatusCode::OK, "{set}");
    assert_eq!(set, off);
    let audit = store
        .list_audit_for_workspace(room.ws, 50)
        .await
        .unwrap()
        .into_iter()
        .find(|row| row.action == "workspace.review_children.set")
        .expect("the change is audited");
    assert_eq!(audit.metadata, off);
    let (status, _) = review_children(
        &base,
        &admin,
        room.ws,
        Some(json!({ "review_children": "sometimes" })),
    )
    .await;
    assert!(status.is_client_error(), "an unknown setting is refused");
}

#[tokio::test]
async fn another_workspace_cannot_read_or_set_review_children() {
    let (base, store) = spawn().await;
    let victim = room(store.as_ref(), "victim").await;
    let attacker = room(store.as_ref(), "attacker").await;
    let (_m, victim_admin) = member_with(
        store.as_ref(),
        victim.ws,
        "admin",
        &[capability::TOKEN_ADMIN, capability::WORKSPACE_READ],
    )
    .await;
    let (_m, attacker_admin) = member_with(
        store.as_ref(),
        attacker.ws,
        "admin",
        &[capability::TOKEN_ADMIN, capability::WORKSPACE_READ],
    )
    .await;

    let off = json!({ "review_children": "off" });
    let (status, _) = review_children(&base, &victim_admin, victim.ws, Some(off.clone())).await;
    assert_eq!(status, StatusCode::OK);
    let (_, attacker_own) = review_children(&base, &attacker_admin, attacker.ws, None).await;
    assert_eq!(
        attacker_own,
        json!({ "review_children": "spawn" }),
        "the victim's setting does not reach another workspace"
    );
    for put in [None, Some(json!({ "review_children": "spawn" }))] {
        let (status, body) = review_children(&base, &attacker_admin, victim.ws, put).await;
        assert!(
            status == StatusCode::FORBIDDEN || status == StatusCode::NOT_FOUND,
            "another tenant must be refused, got {status}: {body}"
        );
    }
    let (_, read) = review_children(&base, &victim_admin, victim.ws, None).await;
    assert_eq!(read, off, "the victim's setting holds");
}

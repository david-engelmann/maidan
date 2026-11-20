//! Advice at the land gate. Off by default; when on, a hand-off is sent to
//! the decision model as a packet built from the thread, the answer comes
//! back as judgments with the advice the rules draw from them, and nothing
//! about the authoritative gate changes. A workspace asks with its own key
//! when it stored one.
//!
//! The provider here is a stand-in on a loopback address that records what it
//! was sent and answers with the probabilities a test gives it.

use std::{
    collections::BTreeMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
    time::Duration,
};

use axum::{extract::State, http::HeaderMap, routing::post, Json, Router};
use maidan_artifacts::LocalFsStore;
use maidan_server::{
    land_gate_advisor::LandGateAdvisor, router, test_support, AppState, FederationRuntime,
};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{MemberKind, NewChannel, NewMessage, NewThread, TaskAttributes, ThreadId};
use reqwest::StatusCode;
use serde_json::{json, Value};

const INSTANCE_KEY: &str = "ts_instance_key";
const WORKSPACE_KEY: &str = "ts_workspace_key";

/// What the stand-in provider saw and how it answers.
#[derive(Default)]
struct Provider {
    /// Every `/v1/systemone` call: the bearer it carried and its body.
    asked: Mutex<Vec<(String, Value)>>,
    /// Probabilities by question, over the all-clear defaults.
    answers: Mutex<BTreeMap<String, f64>>,
    /// When set, `/v1/systemone` answers 500.
    down: Mutex<bool>,
}

fn bearer_of(headers: &HeaderMap) -> String {
    headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .trim_start_matches("Bearer ")
        .to_string()
}

async fn systemone(
    State(provider): State<Arc<Provider>>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> (StatusCode, Json<Value>) {
    if *provider.down.lock().unwrap() {
        return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({})));
    }
    provider
        .asked
        .lock()
        .unwrap()
        .push((bearer_of(&headers), body.clone()));
    let given = provider.answers.lock().unwrap().clone();
    let mut answers = serde_json::Map::new();
    for (name, question) in body["questions"].as_object().unwrap() {
        assert_eq!(question["type"], "noul", "{name} is not a yes/no question");
        // All clear: the work is done and tested, and no warning sign shows.
        let clear = match name.as_str() {
            "done" | "tests_passed" | "code_change" => 0.96,
            _ => 0.03,
        };
        let p = given.get(name).copied().unwrap_or(clear);
        answers.insert(name.clone(), json!({ "type": "noul", "noul": p }));
    }
    (
        StatusCode::OK,
        Json(json!({
            "model": "jev-fixture",
            "answers": answers,
            "usage": { "input_tokens": 420, "output_tokens": 10 },
        })),
    )
}

async fn models(headers: HeaderMap) -> (StatusCode, Json<Value>) {
    if [INSTANCE_KEY, WORKSPACE_KEY].contains(&bearer_of(&headers).as_str()) {
        (StatusCode::OK, Json(json!({ "data": [] })))
    } else {
        (StatusCode::UNAUTHORIZED, Json(json!({ "detail": "no" })))
    }
}

struct World {
    addr: SocketAddr,
    store: Arc<dyn Store>,
    provider: Arc<Provider>,
}

impl World {
    fn url(&self, path: &str) -> String {
        format!("http://{}{path}", self.addr)
    }
    fn advice_url(&self, thread: ThreadId) -> String {
        self.url(&format!("/threads/{}/land-gate/advice", thread.0))
    }
    fn asked(&self) -> Vec<(String, Value)> {
        self.provider.asked.lock().unwrap().clone()
    }
    fn answer(&self, id: &str, p: f64) {
        self.provider.answers.lock().unwrap().insert(id.into(), p);
    }
}

/// A server with the advisor on (`Some(key)`: with or without a key of its
/// own) or off (`None`).
#[allow(clippy::disallowed_methods)] // a stand-in provider, not the API
async fn spawn(advisor: Option<Option<&str>>) -> World {
    let provider = Arc::new(Provider::default());
    let provider_app = Router::new()
        .route("/v1/systemone", post(systemone))
        .route("/v1/models", axum::routing::get(models))
        .with_state(provider.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let provider_addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, provider_app).await.unwrap() });

    let pool = maidan_search::sqlite_pool_options()
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
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    let mut state = AppState::for_tests(store.clone(), artifacts, bus, search);
    // Workspace secrets need the at-rest key; set before the advisor copies it.
    state.federation = FederationRuntime::new(true, Some(Arc::new([7u8; 32])));
    if let Some(instance_key) = advisor {
        let advisor = LandGateAdvisor::with_http(
            reqwest::Client::new(),
            format!("http://{provider_addr}/").parse().unwrap(),
            instance_key.map(str::to_string),
            "jev-latest",
            Duration::from_secs(5),
        )
        .unwrap();
        state.attach_land_gate_advisor(Arc::new(advisor));
    }
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    World {
        addr,
        store,
        provider,
    }
}

/// A task an agent has worked: claimed, a message from the agent, a message
/// from someone else, and a result. Returns the thread and the tenant.
async fn handed_off(world: &World, name: &str) -> (ThreadId, test_support::Tenant) {
    handed_off_with(world, name, None).await
}

/// A hand-off on a task whose creator said how a reviewer checks it.
async fn handed_off_with(
    world: &World,
    name: &str,
    acceptance: Option<&str>,
) -> (ThreadId, test_support::Tenant) {
    let store = world.store.as_ref();
    let tenant = test_support::tenant(store, name).await;
    let channel = store
        .create_channel(NewChannel {
            workspace_id: tenant.workspace_id,
            name: "api".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let (thread, _) = store
        .create_thread_with_footprint(
            NewThread {
                channel_id: channel.id,
                parent_thread_id: None,
                title: Some("Rate-limit the public API".into()),
                description: Some("60 requests a minute per token, then 429.".into()),
                created_by: Some(tenant.admin_id),
            },
            None,
            acceptance.map(|acceptance| TaskAttributes {
                acceptance: Some(acceptance.into()),
                ..TaskAttributes::default()
            }),
            None,
        )
        .await
        .unwrap();
    let agent = tenant.member(store, "builder", MemberKind::Agent).await;
    let claim = store
        .claim_thread(thread.id, agent.member_id)
        .await
        .unwrap();
    assert!(claim.claimed);
    for (author, body) in [
        (agent.member_id, "The limiter is in and the tests pass."),
        (tenant.admin_id, "Thanks, I will look this afternoon."),
    ] {
        store
            .post_message(NewMessage {
                thread_id: thread.id,
                author_id: author,
                body: body.into(),
                metadata: json!({}),
                content: None,
            })
            .await
            .unwrap();
    }
    store
        .set_thread_result(
            thread.id,
            agent.member_id,
            claim.thread.claim_lease_id.unwrap(),
            &json!({
                "status": "changed",
                "summary": "Adds a token-bucket limiter; 14 new tests pass.",
                "diff": "diff --git a/src/limit.rs b/src/limit.rs\n+fn limit() {}",
                "deliver_to": { "repo": "example/app" },
            }),
        )
        .await
        .unwrap();
    (thread.id, tenant)
}

async fn gate_is_untouched(world: &World, thread: ThreadId) {
    let standing = world.store.get_land_gate_standing(thread).await.unwrap();
    assert!(!standing.required);
    assert!(standing.pointer.is_none());
}

#[tokio::test]
async fn the_advisor_is_off_by_default() {
    let world = spawn(None).await;
    let (thread, tenant) = handed_off(&world, "off").await;
    let client = tenant.client();
    let advice = client
        .post(world.advice_url(thread))
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(advice.status(), StatusCode::NOT_FOUND);
    let status: Value = client
        .get(world.url(&format!(
            "/workspaces/{}/land-gate/advisor",
            tenant.workspace_id.0
        )))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status, json!({ "enabled": false }));
    let set = client
        .put(world.url(&format!(
            "/workspaces/{}/land-gate/advisor",
            tenant.workspace_id.0
        )))
        .json(&json!({ "api_key": WORKSPACE_KEY }))
        .send()
        .await
        .unwrap();
    assert_eq!(set.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_hand_off_is_sent_as_a_packet_and_answered_with_judgments() {
    let world = spawn(Some(Some(INSTANCE_KEY))).await;
    let (thread, tenant) = handed_off(&world, "packet").await;
    let client = tenant.client();
    let response = client
        .post(world.advice_url(thread))
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let advice: Value = response.json().await.unwrap();
    assert_eq!(advice["land"], "green", "{advice}");
    assert_eq!(advice["reasons"], json!([]));
    assert_eq!(advice["provider"], "typesafe");
    assert_eq!(advice["model"], "jev-fixture");
    assert_eq!(advice["key_source"], "instance");
    assert_eq!(advice["cached"], false);
    assert_eq!(
        advice["thresholds"],
        json!({ "yes_min": 0.7, "no_max": 0.3 })
    );
    assert_eq!(advice["usage"]["input_tokens"], 420);
    let judgments = advice["judgments"].as_array().unwrap();
    assert_eq!(judgments.len(), 10);
    let done = &judgments[0];
    assert_eq!(
        (&done["id"], &done["answer"], &done["concern"]),
        (&json!("done"), &json!("yes"), &json!(false))
    );
    assert!(judgments.iter().all(|j| j["label"].as_str().is_some()));
    assert_eq!(done["reading"], "Does what the task asks");
    let failing = judgments
        .iter()
        .find(|j| j["id"] == "unresolved_failure")
        .unwrap();
    assert_eq!(failing["reading"], "Reports nothing still failing");

    // The packet: what the task asks, the result without its diff, and only
    // what the agent that produced the result said.
    let asked = world.asked();
    assert_eq!(asked.len(), 1);
    let (bearer, body) = &asked[0];
    assert_eq!(bearer, INSTANCE_KEY);
    assert_eq!(body["model"], "jev-latest");
    assert_eq!(
        body["state"],
        json!({
            "task": {
                "title": "Rate-limit the public API",
                "description": "60 requests a minute per token, then 429.",
            },
            "result": {
                "status": "changed",
                "summary": "Adds a token-bucket limiter; 14 new tests pass.",
            },
            "agent_messages": ["The limiter is in and the tests pass."],
        })
    );
    let questions = body["questions"].as_object().unwrap();
    assert_eq!(questions.len(), 10);
    for (name, question) in questions {
        assert!(question["instructions"].as_str().unwrap().ends_with('?'));
        assert!(question["criteria"]["true"].is_string(), "{name}");
        assert!(question["criteria"]["false"].is_string(), "{name}");
    }
    gate_is_untouched(&world, thread).await;

    // The same hand-off again is answered from the hold; refresh asks again.
    let again: Value = client
        .post(world.advice_url(thread))
        .json(&json!({}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(again["cached"], true);
    assert_eq!(world.asked().len(), 1);
    let fresh: Value = client
        .post(world.advice_url(thread))
        .json(&json!({ "refresh": true }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(fresh["cached"], false);
    assert_eq!(world.asked().len(), 2);
}

#[tokio::test]
async fn a_tasks_acceptance_is_sent_with_what_it_asks_for() {
    let world = spawn(Some(Some(INSTANCE_KEY))).await;
    let acceptance = "cargo test passes; the 61st request in a minute gets 429.";
    let (thread, tenant) = handed_off_with(&world, "acceptance", Some(acceptance)).await;
    let response = tenant
        .client()
        .post(world.advice_url(thread))
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let asked = world.asked();
    assert_eq!(asked.len(), 1);
    assert_eq!(
        asked[0].1["state"]["task"],
        json!({
            "title": "Rate-limit the public API",
            "description": "60 requests a minute per token, then 429.",
            "acceptance": acceptance,
        })
    );
}

#[tokio::test]
async fn the_rules_read_the_judgments_and_an_unsettled_one_is_never_green() {
    let world = spawn(Some(Some(INSTANCE_KEY))).await;
    let (thread, tenant) = handed_off(&world, "rules").await;
    let client = tenant.client();
    let advise = || async {
        let advice: Value = client
            .post(world.advice_url(thread))
            .json(&json!({ "refresh": true }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        advice
    };

    world.answer("flags_risk", 0.5);
    let advice = advise().await;
    assert_eq!(advice["land"], "amber", "{advice}");
    assert_eq!(advice["reasons"], json!(["flags_risk"]));
    let risk = advice["judgments"]
        .as_array()
        .unwrap()
        .iter()
        .find(|j| j["id"] == "flags_risk")
        .unwrap();
    assert_eq!(
        (&risk["answer"], &risk["concern"]),
        (&json!("unsettled"), &json!(true))
    );
    assert_eq!(risk["reading"], "Unclear whether the agent flags a risk");

    world.answer("flags_risk", 0.03);
    world.answer("tests_passed", 0.02);
    let advice = advise().await;
    assert_eq!(advice["land"], "amber");
    assert_eq!(advice["reasons"], json!(["tests_not_reported"]));

    world.answer("unresolved_failure", 0.98);
    let advice = advise().await;
    assert_eq!(advice["land"], "red");
    assert_eq!(
        advice["reasons"],
        json!(["unresolved_failure", "tests_not_reported"]),
        "what made it red leads, and the other signs follow"
    );
    gate_is_untouched(&world, thread).await;
}

#[tokio::test]
async fn a_task_with_no_result_has_nothing_to_advise_on() {
    let world = spawn(Some(Some(INSTANCE_KEY))).await;
    let store = world.store.as_ref();
    let tenant = test_support::tenant(store, "empty").await;
    let channel = store
        .create_channel(NewChannel {
            workspace_id: tenant.workspace_id,
            name: "api".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("Not started".into()),
            description: None,
            created_by: None,
        })
        .await
        .unwrap();
    let response = tenant
        .client()
        .post(world.advice_url(thread.id))
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::CONFLICT);
    assert!(world.asked().is_empty(), "nothing left for the provider");
}

#[tokio::test]
async fn a_workspace_asks_with_its_own_key_and_another_cannot_set_it() {
    // The instance has no key of its own: workspaces bring theirs.
    let world = spawn(Some(None)).await;
    let (thread, tenant) = handed_off(&world, "own-key").await;
    let other = test_support::tenant(world.store.as_ref(), "other").await;
    let client = tenant.client();
    let advisor = world.url(&format!(
        "/workspaces/{}/land-gate/advisor",
        tenant.workspace_id.0
    ));

    let status: Value = client
        .get(&advisor)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status["enabled"], true);
    assert!(status.get("key_source").is_none(), "{status}");
    assert_eq!(status["model"], "jev-latest");
    let refused = client
        .post(world.advice_url(thread))
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::CONFLICT, "no key yet");
    assert!(world.asked().is_empty());

    // Another workspace's admin cannot set this one's key.
    let foreign = other
        .client()
        .put(&advisor)
        .json(&json!({ "api_key": WORKSPACE_KEY }))
        .send()
        .await
        .unwrap();
    assert_eq!(foreign.status(), StatusCode::FORBIDDEN);

    // A key the provider refuses is not stored.
    let bad = client
        .put(&advisor)
        .json(&json!({ "api_key": "ts_wrong" }))
        .send()
        .await
        .unwrap();
    assert_eq!(bad.status(), StatusCode::BAD_REQUEST);
    let spaced = client
        .put(&advisor)
        .json(&json!({ "api_key": "ts with spaces" }))
        .send()
        .await
        .unwrap();
    assert_eq!(spaced.status(), StatusCode::BAD_REQUEST);

    let set = client
        .put(&advisor)
        .json(&json!({ "api_key": WORKSPACE_KEY }))
        .send()
        .await
        .unwrap();
    assert_eq!(set.status(), StatusCode::OK);
    let status: Value = set.json().await.unwrap();
    assert_eq!(status["key_source"], "workspace");
    assert!(!status.to_string().contains(WORKSPACE_KEY), "never the key");

    let advice: Value = client
        .post(world.advice_url(thread))
        .json(&json!({}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(advice["key_source"], "workspace", "{advice}");
    assert_eq!(world.asked()[0].0, WORKSPACE_KEY);

    // The other workspace still has no key and gets no advice from this one's.
    let (their_thread, _) = (thread, ());
    let theirs = other
        .client()
        .post(world.advice_url(their_thread))
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert!(
        theirs.status() == StatusCode::FORBIDDEN || theirs.status() == StatusCode::NOT_FOUND,
        "another workspace cannot read this thread: {}",
        theirs.status()
    );

    let cleared = client.delete(&advisor).send().await.unwrap();
    assert_eq!(cleared.status(), StatusCode::OK);
    let status: Value = cleared.json().await.unwrap();
    assert!(status.get("key_source").is_none(), "{status}");
    let refused = client
        .post(world.advice_url(thread))
        .json(&json!({ "refresh": true }))
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::CONFLICT, "the key is gone");
}

#[tokio::test]
async fn the_workspaces_key_is_out_of_reach_of_the_generic_secret_routes() {
    let world = spawn(Some(None)).await;
    let (_thread, tenant) = handed_off(&world, "reserved").await;
    let client = tenant.client();
    let ws = tenant.workspace_id.0;
    let advisor = world.url(&format!("/workspaces/{ws}/land-gate/advisor"));
    let set = client
        .put(&advisor)
        .json(&json!({ "api_key": WORKSPACE_KEY }))
        .send()
        .await
        .unwrap();
    assert_eq!(set.status(), StatusCode::OK);

    // The name the key is stored under is one the server keeps for itself.
    let name = "maidan:typesafe_api_key";
    let listed = client
        .get(world.url(&format!("/workspaces/{ws}/secrets")))
        .send()
        .await
        .unwrap();
    assert_eq!(listed.status(), StatusCode::OK);
    let listed = listed.text().await.unwrap();
    assert!(!listed.contains("typesafe"), "not listed: {listed}");

    let resolved = client
        .post(world.url(&format!("/workspaces/{ws}/secrets/{name}/resolve")))
        .send()
        .await
        .unwrap();
    assert_eq!(resolved.status(), StatusCode::NOT_FOUND);
    assert!(!resolved.text().await.unwrap().contains(WORKSPACE_KEY));

    let deleted = client
        .delete(world.url(&format!("/workspaces/{ws}/secrets/{name}")))
        .send()
        .await
        .unwrap();
    assert_eq!(deleted.status(), StatusCode::NOT_FOUND);

    // A value the provider never accepted cannot be written over it.
    let overwritten = client
        .post(world.url(&format!("/workspaces/{ws}/secrets")))
        .json(&json!({ "name": name, "value": "ts_wrong" }))
        .send()
        .await
        .unwrap();
    assert_eq!(overwritten.status(), StatusCode::BAD_REQUEST);

    // The MCP tools answer the same way.
    for (tool, arguments) in [
        ("resolve_secret", json!({ "name": name })),
        ("delete_secret", json!({ "name": name })),
        (
            "create_secret",
            json!({ "name": name, "value": "ts_wrong" }),
        ),
        ("list_secrets", json!({})),
    ] {
        let answer = client
            .post(world.url("/mcp"))
            .json(&json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "tools/call",
                "params": { "name": tool, "arguments": arguments },
            }))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert!(!answer.contains(WORKSPACE_KEY), "{tool}: {answer}");
        if tool == "list_secrets" {
            assert!(!answer.contains("typesafe"), "{tool}: {answer}");
        } else {
            let answer: Value = serde_json::from_str(&answer).unwrap();
            assert!(
                answer.get("error").is_some() || answer["result"]["isError"] == true,
                "{tool} must refuse: {answer}"
            );
        }
    }

    // The key is still there, still the workspace's, and still the one used.
    let status: Value = client
        .get(&advisor)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(status["key_source"], "workspace");
}

#[tokio::test]
async fn the_opening_message_stays_out_even_when_its_author_did_the_work() {
    let world = spawn(Some(Some(INSTANCE_KEY))).await;
    let store = world.store.as_ref();
    let tenant = test_support::tenant(store, "opening").await;
    let channel = store
        .create_channel(NewChannel {
            workspace_id: tenant.workspace_id,
            name: "api".into(),
            topic: None,
            private: false,
        })
        .await
        .unwrap();
    // The agent files the task itself, writes its opening message, then does
    // the work: the first message is the ask, and only what follows is its
    // account of the work.
    let agent = tenant.member(store, "builder", MemberKind::Agent).await;
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("Rename the billing module".into()),
            description: None,
            created_by: Some(agent.member_id),
        })
        .await
        .unwrap();
    let post = |body: &'static str| {
        store.post_message(NewMessage {
            thread_id: thread.id,
            author_id: agent.member_id,
            body: body.into(),
            metadata: json!({}),
            content: None,
        })
    };
    post("Claim this, rename crates/billing, say what you ran, send it to review.")
        .await
        .unwrap();
    let claim = store
        .claim_thread(thread.id, agent.member_id)
        .await
        .unwrap();
    assert!(claim.claimed);
    post("Renamed the crate and every import.").await.unwrap();
    post("cargo test passes.").await.unwrap();
    store
        .set_thread_result(
            thread.id,
            agent.member_id,
            claim.thread.claim_lease_id.unwrap(),
            &json!({ "summary": "Renamed crates/billing to crates/invoicing." }),
        )
        .await
        .unwrap();
    let response = tenant
        .client()
        .post(world.advice_url(thread.id))
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let asked = world.asked();
    assert_eq!(
        asked[0].1["state"]["agent_messages"],
        json!(["Renamed the crate and every import.", "cargo test passes."])
    );
}

#[tokio::test]
async fn a_provider_failure_fails_the_advice_and_leaves_the_gate_alone() {
    let world = spawn(Some(Some(INSTANCE_KEY))).await;
    let (thread, tenant) = handed_off(&world, "down").await;
    *world.provider.down.lock().unwrap() = true;
    let response = tenant
        .client()
        .post(world.advice_url(thread))
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let body: Value = response.json().await.unwrap();
    assert!(body["detail"]
        .as_str()
        .unwrap()
        .contains("authoritative gate is unchanged"));
    gate_is_untouched(&world, thread).await;
}

#[tokio::test]
async fn the_mcp_tool_returns_the_same_advice() {
    let world = spawn(Some(Some(INSTANCE_KEY))).await;
    let (thread, tenant) = handed_off(&world, "mcp").await;
    world.answer("asks_person", 0.9);
    let answer: Value = tenant
        .client()
        .post(world.url("/mcp"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": "advise_land_gate", "arguments": { "thread_id": thread.0 } },
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let text = answer["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("no content: {answer}"));
    let advice: Value = serde_json::from_str(text).unwrap();
    assert_eq!(advice["land"], "amber", "{advice}");
    assert_eq!(advice["reasons"], json!(["asks_person"]));
    assert_eq!(world.asked().len(), 1);
    gate_is_untouched(&world, thread).await;
}

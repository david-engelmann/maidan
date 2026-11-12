//! The demo's per-visitor sandbox (`POST /sandbox`, `MAIDAN_DEMO_SANDBOX`).
//!
//! A stranger asks and gets a workspace of their own: a `#general` channel, a
//! "Say hello" task, an admin token and an agent token. Two sandboxes are two
//! tenants, so each one's tokens see nothing of the other. The mint is off
//! unless the instance turned it on, refuses another site's browser, stops at
//! its cap, and is charged per address.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use maidan_artifacts::LocalFsStore;
use maidan_server::{router, sandbox::SandboxSettings, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use reqwest::StatusCode;
use serde_json::{json, Value};

async fn spawn(sandbox: Option<i64>) -> String {
    // The limiter's buckets are process-global, so each test steers itself
    // into its own address bucket with forwarded addresses.
    // SAFETY: test setup. Maidan reads this variable only through std,
    // whose environment lock serializes the read with this write.
    unsafe { std::env::set_var("MAIDAN_TRUSTED_PROXY_HOPS", "1") };
    let pool = maidan_search::sqlite_pool_options()
        .max_connections(4)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
    let search: Arc<dyn maidan_search::Search> =
        Arc::new(maidan_search::SqliteSearch::new(pool.clone()));
    let dir = tempfile::tempdir().unwrap();
    let mut state = AppState::new(
        store,
        Arc::new(LocalFsStore::new(dir.path())),
        Arc::new(maidan_bus::InMemoryBus::new()),
        search,
        Arc::new(maidan_search::HashV1Provider),
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    state.sandbox = sandbox.map(|max_sandboxes| {
        Arc::new(SandboxSettings {
            max_sandboxes,
            mcp_url: "https://try.example/mcp/streamable".into(),
            console_url: "https://try.example/ui/".into(),
        })
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let app = router(state);
    tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap()
    });
    std::mem::forget(dir);
    format!("http://{addr}")
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .build()
        .unwrap()
}

async fn mint(client: &reqwest::Client, base: &str, ip: &str) -> reqwest::Response {
    client
        .post(format!("{base}/sandbox"))
        .header("x-forwarded-for", ip)
        .json(&json!({}))
        .send()
        .await
        .unwrap()
}

async fn minted(client: &reqwest::Client, base: &str, ip: &str) -> Value {
    let res = mint(client, base, ip).await;
    assert_eq!(res.status(), StatusCode::CREATED);
    assert_eq!(
        res.headers()
            .get(reqwest::header::CACHE_CONTROL)
            .and_then(|v| v.to_str().ok()),
        Some("no-store"),
        "the answer holds two secrets"
    );
    res.json().await.unwrap()
}

fn s<'a>(v: &'a Value, path: &[&str]) -> &'a str {
    let mut at = v;
    for key in path {
        at = &at[*key];
    }
    at.as_str()
        .unwrap_or_else(|| panic!("{path:?} is not a string in {v}"))
}

async fn get(
    client: &reqwest::Client,
    base: &str,
    token: &str,
    path: &str,
) -> (StatusCode, String) {
    let res = client
        .get(format!("{base}{path}"))
        .bearer_auth(token)
        .send()
        .await
        .unwrap();
    let status = res.status();
    (status, res.text().await.unwrap_or_default())
}

async fn tool(client: &reqwest::Client, base: &str, token: &str, name: &str, args: Value) -> Value {
    let body: Value = client
        .post(format!("{base}/mcp"))
        .bearer_auth(token)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": { "name": name, "arguments": args }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        body.get("error").is_none() && body["result"]["isError"] != json!(true),
        "{name} failed: {body}"
    );
    let text = body["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or("null");
    serde_json::from_str(text).unwrap_or(Value::Null)
}

#[tokio::test]
async fn a_sandbox_opens_on_general_with_the_say_hello_task_and_expiring_tokens() {
    let base = spawn(Some(10)).await;
    let c = client();
    let sb = minted(&c, &base, "10.1.0.1").await;
    assert_eq!(s(&sb, &["mcp_url"]), "https://try.example/mcp/streamable");
    let expires: chrono::DateTime<chrono::Utc> = s(&sb, &["expires_at"]).parse().unwrap();
    let hours = (expires - chrono::Utc::now()).num_minutes() as f64 / 60.0;
    assert!(
        (23.9..=24.0).contains(&hours),
        "tokens live 24 hours, not {hours}"
    );

    let admin = s(&sb, &["admin", "token"]);
    let ws = s(&sb, &["workspace_id"]);
    let channel = s(&sb, &["channel_id"]);
    let task = s(&sb, &["task_id"]);
    let (status, channels) = get(&c, &base, admin, &format!("/workspaces/{ws}/channels")).await;
    assert_eq!(status, StatusCode::OK, "{channels}");
    let channels: Value = serde_json::from_str(&channels).unwrap();
    assert!(channels
        .as_array()
        .unwrap()
        .iter()
        .any(|ch| ch["id"] == json!(channel) && ch["name"] == json!("general")));
    let (status, thread) = get(&c, &base, admin, &format!("/threads/{task}")).await;
    assert_eq!(status, StatusCode::OK, "{thread}");
    let thread: Value = serde_json::from_str(&thread).unwrap();
    // The sign-up's starter task, word for word: one task, worded once.
    assert_eq!(thread["title"], json!("Say hello"), "{thread}");
    assert!(
        thread["description"]
            .as_str()
            .is_some_and(|d| d.starts_with("A first task for the agent you connect")),
        "{thread}"
    );

    // Its opening message walks the agent through the loop, as on sign-up.
    let (status, messages) = get(&c, &base, admin, &format!("/threads/{task}/messages")).await;
    assert_eq!(status, StatusCode::OK, "{messages}");
    let messages: Value = serde_json::from_str(&messages).unwrap();
    let bodies: Vec<&str> = messages
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|m| m["body"].as_str())
        .collect();
    assert_eq!(bodies.len(), 1, "one opening message: {messages}");
    assert!(bodies[0].starts_with("Welcome to Maidan."), "{messages}");
    for step in [
        "claim_next_thread",
        "post_message",
        "set_thread_result",
        "transition_thread",
    ] {
        assert!(
            bodies[0].contains(step),
            "the message names `{step}`: {}",
            bodies[0]
        );
    }
}

#[tokio::test]
async fn the_agent_token_runs_the_say_hello_task_to_review() {
    let base = spawn(Some(10)).await;
    let c = client();
    let sb = minted(&c, &base, "10.2.0.1").await;
    let agent = s(&sb, &["agent", "token"]);
    let channel = s(&sb, &["channel_id"]);
    let task = s(&sb, &["task_id"]);

    let claim = tool(
        &c,
        &base,
        agent,
        "claim_next_thread",
        json!({ "channel_id": channel }),
    )
    .await;
    assert_eq!(
        claim["id"],
        json!(task),
        "the starter task is the one to claim: {claim}"
    );
    tool(
        &c,
        &base,
        agent,
        "post_message",
        json!({ "thread_id": task, "body": "hello" }),
    )
    .await;
    tool(
        &c,
        &base,
        agent,
        "set_thread_result",
        json!({ "thread_id": task, "claim_lease_id": claim["claim_lease_id"], "result": { "said": "hello" } }),
    )
    .await;
    let reviewed = tool(
        &c,
        &base,
        agent,
        "transition_thread",
        json!({ "thread_id": task, "action": "start_review" }),
    )
    .await;
    assert_eq!(reviewed["state"], json!("in_review"), "{reviewed}");
}

#[tokio::test]
async fn one_sandboxs_tokens_see_nothing_of_another_sandbox() {
    let base = spawn(Some(10)).await;
    let c = client();
    let a = minted(&c, &base, "10.3.0.1").await;
    let b = minted(&c, &base, "10.3.0.2").await;
    assert_ne!(a["workspace_id"], b["workspace_id"]);

    let b_ws = s(&b, &["workspace_id"]);
    let b_channel = s(&b, &["channel_id"]);
    let b_task = s(&b, &["task_id"]);
    let b_agent_id = s(&b, &["agent", "member_id"]);
    let probes = [
        format!("/workspaces/{b_ws}"),
        format!("/workspaces/{b_ws}/channels"),
        format!("/workspaces/{b_ws}/members"),
        format!("/channels/{b_channel}/threads"),
        format!("/threads/{b_task}"),
        format!("/threads/{b_task}/messages"),
        format!("/members/{b_agent_id}"),
    ];
    for token in [s(&a, &["admin", "token"]), s(&a, &["agent", "token"])] {
        for path in &probes {
            let (status, body) = get(&c, &base, token, path).await;
            assert!(
                !status.is_success(),
                "sandbox A read {path} of sandbox B: {status} {body}"
            );
            assert!(!body.contains("Say hello"), "{path}: {body}");
        }
        let res = c
            .post(format!("{base}/threads/{b_task}/messages"))
            .bearer_auth(token)
            .json(&json!({ "body": "hello from A" }))
            .send()
            .await
            .unwrap();
        assert!(
            !res.status().is_success(),
            "A posted into B: {}",
            res.status()
        );
        // Over MCP too: B's channel has no work for A's agent.
        let body: Value = c
            .post(format!("{base}/mcp"))
            .bearer_auth(token)
            .json(&json!({
                "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": { "name": "claim_next_thread", "arguments": { "channel_id": b_channel } }
            }))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
        let refused = body.get("error").is_some() || body["result"]["isError"] == json!(true);
        let empty = body["result"]["content"][0]["text"].as_str() == Some("null");
        assert!(refused || empty, "A claimed in B: {body}");
    }
    // B is untouched: its task is still open and unclaimed.
    let (status, thread) = get(
        &c,
        &base,
        s(&b, &["admin", "token"]),
        &format!("/threads/{b_task}"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let thread: Value = serde_json::from_str(&thread).unwrap();
    assert_eq!(thread["state"], json!("open"), "{thread}");
    assert!(thread["assignee_id"].is_null(), "{thread}");
}

#[tokio::test]
async fn the_mint_answers_404_unless_the_instance_turned_it_on() {
    let base = spawn(None).await;
    assert_eq!(
        mint(&client(), &base, "10.4.0.1").await.status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn a_fourth_sandbox_from_one_address_in_an_hour_is_refused() {
    let base = spawn(Some(10)).await;
    let c = client();
    for _ in 0..3 {
        minted(&c, &base, "10.5.0.1").await;
    }
    assert_eq!(
        mint(&c, &base, "10.5.0.1").await.status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    // Another address still gets one.
    minted(&c, &base, "10.5.0.2").await;
}

#[tokio::test]
async fn the_mint_stops_at_its_cap() {
    let base = spawn(Some(1)).await;
    let c = client();
    minted(&c, &base, "10.6.0.1").await;
    assert_eq!(
        mint(&c, &base, "10.6.0.2").await.status(),
        StatusCode::SERVICE_UNAVAILABLE
    );
}

#[tokio::test]
async fn another_sites_browser_cannot_start_a_sandbox() {
    let base = spawn(Some(10)).await;
    let res = client()
        .post(format!("{base}/sandbox"))
        .header("x-forwarded-for", "10.7.0.1")
        .header("origin", "https://elsewhere.example")
        .json(&json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);
}

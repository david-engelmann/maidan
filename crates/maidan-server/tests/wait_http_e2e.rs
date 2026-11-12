//! `wait_for_*` tools work over HTTP MCP.
//!
//! The HTTP server built its MCP server with no event bus, so every wait tool
//! answered "requires an event bus". The server's bus is now wired in: a wait
//! over `POST /mcp` wakes when the awaited event is published, on one replica
//! and across two, and a wait never wakes on another workspace's event.

mod common;

use std::{net::SocketAddr, sync::Arc, time::Duration};

use maidan_artifacts::LocalFsStore;
use maidan_bus::{EventBus, InMemoryBus, PostgresBus};
use maidan_server::{router, test_support, AppState};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::MemberKind;
use serde_json::{json, Value};
use sqlx::PgPool;

struct Harness {
    addr: SocketAddr,
    store: Arc<dyn Store>,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.server.abort();
    }
}

/// One replica over SQLite with an in-memory bus.
async fn spawn_sqlite() -> (Harness, Arc<dyn EventBus>) {
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
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus: Arc<dyn EventBus> = Arc::new(InMemoryBus::new());
    let mut state = AppState::for_tests(store.clone(), artifacts, bus.clone(), search);
    state.subscribe_resume_secret = Some(Arc::from(b"test-secret".as_slice()));
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (
        Harness {
            addr,
            store,
            server,
        },
        bus,
    )
}

/// One replica over Postgres with a NOTIFY bus, sharing the database.
async fn spawn_postgres_replica(
    pool: &PgPool,
    artifacts: &std::path::Path,
) -> (Harness, Arc<PostgresBus>) {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(6)
        .acquire_timeout(Duration::from_secs(15))
        .connect_with((*pool.connect_options()).clone())
        .await
        .expect("replica pool");
    let store: Arc<dyn Store> = Arc::new(PostgresStore::for_tests(pool.clone()));
    let search: Arc<dyn maidan_search::Search> =
        Arc::new(maidan_search::PostgresSearch::new(pool.clone()));
    let artifacts = Arc::new(LocalFsStore::new(artifacts));
    let bus = Arc::new(
        PostgresBus::connect(pool.clone(), maidan_store::test_support::dev_keys())
            .await
            .expect("connect the bus"),
    );
    // Let the LISTEN task subscribe before events flow.
    tokio::time::sleep(Duration::from_millis(200)).await;
    let mut state = AppState::for_tests(store.clone(), artifacts, bus.clone(), search);
    state.subscribe_resume_secret = Some(Arc::from(b"test-secret".as_slice()));
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (
        Harness {
            addr,
            store,
            server,
        },
        bus,
    )
}

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap()
}

/// Claim `thread_id` as the holder of `token` over MCP and return the lease a
/// result write must name.
async fn claim_lease(client: &reqwest::Client, base: &str, token: &str, thread_id: &str) -> Value {
    let resp = mcp_call(
        client,
        base,
        token,
        "claim_thread",
        json!({"thread_id": thread_id}),
    )
    .await;
    let text = resp["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_default();
    let claimed: Value = serde_json::from_str(text).unwrap_or(Value::Null);
    let lease = claimed["thread"]["claim_lease_id"].clone();
    assert!(lease.is_string(), "claim_thread: {resp}");
    lease
}

/// Call an MCP tool over `POST /mcp` as the member `token` belongs to.
async fn mcp_call(
    client: &reqwest::Client,
    base: &str,
    token: &str,
    name: &str,
    arguments: Value,
) -> Value {
    client
        .post(format!("{base}/mcp"))
        .header("content-type", "application/json")
        .header("accept", "application/json")
        .bearer_auth(token)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/call",
            "params": {"name": name, "arguments": arguments},
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap()
}

/// The tool's JSON payload out of a `tools/call` response.
fn tool_text(resp: &Value) -> Value {
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    serde_json::from_str(text).unwrap()
}

/// A workspace with one member, one channel and one thread, over REST.
/// Returns (workspace_id, the member's token, thread_id).
async fn workspace_with_thread(
    store: &dyn Store,
    base: &str,
    ws_name: &str,
) -> (String, String, String) {
    let tenant = test_support::tenant(store, ws_name).await;
    let client = &tenant.client();
    let workspace_id = tenant.workspace_id.0.to_string();
    let waiter = tenant.member(store, "waiter", MemberKind::Agent).await;
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
        .json(&json!({"title": "wait-target"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let thread_id = th["id"].as_str().unwrap().to_string();
    (workspace_id, waiter.token, thread_id)
}

#[tokio::test]
async fn http_wait_for_result_wakes_when_the_result_is_set() {
    let (h, _bus) = spawn_sqlite().await;
    let base = format!("http://{}", h.addr);
    let client = client();
    let (_ws, token, thread_id) = workspace_with_thread(h.store.as_ref(), &base, "wait-ws").await;

    // The wait parks on the server's event bus; run it in the background.
    let wait_client = client.clone();
    let wait_base = base.clone();
    let wait_token = token.clone();
    let wait_thread = thread_id.clone();
    let waiter = tokio::spawn(async move {
        mcp_call(
            &wait_client,
            &wait_base,
            &wait_token,
            "wait_for_result",
            json!({"thread_id": wait_thread, "timeout_ms": 10000}),
        )
        .await
    });
    // Let the wait subscribe before the result lands.
    tokio::time::sleep(Duration::from_millis(500)).await;

    let lease = claim_lease(&client, &base, &token, &thread_id).await;
    let set_resp = mcp_call(
        &client,
        &base,
        &token,
        "set_thread_result",
        json!({"thread_id": thread_id, "result": {"answer": 42}, "claim_lease_id": lease}),
    )
    .await;
    assert!(set_resp.get("error").is_none(), "{set_resp}");

    let wait_resp = tokio::time::timeout(Duration::from_secs(15), waiter)
        .await
        .expect("waiter finishes")
        .expect("waiter joins");
    assert!(wait_resp.get("error").is_none(), "{wait_resp}");
    assert_eq!(tool_text(&wait_resp)["result"], json!({"answer": 42}));
}

#[tokio::test]
async fn http_wait_for_result_wakes_across_two_replicas() {
    let Some((container, pool)) = common::postgres_pool().await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let (a, _bus_a) = spawn_postgres_replica(&pool, dir.path()).await;
    let (b, _bus_b) = spawn_postgres_replica(&pool, dir.path()).await;
    let base_a = format!("http://{}", a.addr);
    let base_b = format!("http://{}", b.addr);
    let client = client();
    let (_ws, token, thread_id) =
        workspace_with_thread(a.store.as_ref(), &base_a, "wait-2r-ws").await;

    // Wait on replica A; the result is set through replica B.
    let wait_client = client.clone();
    let wait_token = token.clone();
    let wait_thread = thread_id.clone();
    let waiter = tokio::spawn(async move {
        mcp_call(
            &wait_client,
            &base_a,
            &wait_token,
            "wait_for_result",
            json!({"thread_id": wait_thread, "timeout_ms": 10000}),
        )
        .await
    });
    tokio::time::sleep(Duration::from_millis(500)).await;

    let lease = claim_lease(&client, &base_b, &token, &thread_id).await;
    let set_resp = mcp_call(
        &client,
        &base_b,
        &token,
        "set_thread_result",
        json!({"thread_id": thread_id, "result": {"from": "replica-b"}, "claim_lease_id": lease}),
    )
    .await;
    assert!(set_resp.get("error").is_none(), "{set_resp}");

    let wait_resp = tokio::time::timeout(Duration::from_secs(15), waiter)
        .await
        .expect("waiter finishes")
        .expect("waiter joins");
    assert!(wait_resp.get("error").is_none(), "{wait_resp}");
    assert_eq!(
        tool_text(&wait_resp)["result"],
        json!({"from": "replica-b"})
    );
    drop(container);
}

#[tokio::test]
async fn http_wait_never_wakes_on_another_workspace_event() {
    let (h, _bus) = spawn_sqlite().await;
    let base = format!("http://{}", h.addr);
    let client = client();
    let (_ws_a, token_a, thread_a) =
        workspace_with_thread(h.store.as_ref(), &base, "wait-tenant-a").await;
    let (_ws_b, token_b, thread_b) =
        workspace_with_thread(h.store.as_ref(), &base, "wait-tenant-b").await;

    // Wait on A's thread; the event fires on B's thread.
    let wait_client = client.clone();
    let wait_base = base.clone();
    let waiter = tokio::spawn(async move {
        mcp_call(
            &wait_client,
            &wait_base,
            &token_a,
            "wait_for_result",
            json!({"thread_id": thread_a, "timeout_ms": 1000}),
        )
        .await
    });
    tokio::time::sleep(Duration::from_millis(500)).await;

    let lease = claim_lease(&client, &base, &token_b, &thread_b).await;
    let set_resp = mcp_call(
        &client,
        &base,
        &token_b,
        "set_thread_result",
        json!({"thread_id": thread_b, "result": {"other": "workspace"}, "claim_lease_id": lease}),
    )
    .await;
    assert!(set_resp.get("error").is_none(), "{set_resp}");

    // The wait must time out (null result), not wake on B's event.
    let wait_resp = tokio::time::timeout(Duration::from_secs(15), waiter)
        .await
        .expect("waiter finishes")
        .expect("waiter joins");
    assert!(wait_resp.get("error").is_none(), "{wait_resp}");
    assert_eq!(tool_text(&wait_resp), Value::Null);
}

/// A workspace with one member and a parent+dependency thread pair, over REST.
/// Returns (the member's token, parent_thread_id, dep_thread_id).
async fn workspace_with_dep_pair(
    store: &dyn Store,
    base: &str,
    ws_name: &str,
) -> (String, String, String) {
    let tenant = test_support::tenant(store, ws_name).await;
    let client = &tenant.client();
    let workspace_id = tenant.workspace_id.0.to_string();
    let waiter = tenant.member(store, "waiter", MemberKind::Agent).await;
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
    let mk_thread = |title: &str| {
        let client = client.clone();
        let base = base.to_string();
        let channel_id = channel_id.to_string();
        let title = title.to_string();
        async move {
            client
                .post(format!("{base}/channels/{channel_id}/threads"))
                .json(&json!({"title": title}))
                .send()
                .await
                .unwrap()
                .json::<Value>()
                .await
                .unwrap()["id"]
                .as_str()
                .unwrap()
                .to_string()
        }
    };
    let parent = mk_thread("parent").await;
    let dep = mk_thread("dep").await;
    (waiter.token, parent, dep)
}

/// Drive a dependency to terminal over MCP, emitting `ThreadReady` for the parent.
async fn close_dependency(
    client: &reqwest::Client,
    base: &str,
    token: &str,
    parent: &str,
    dep: &str,
) {
    for (tool, args) in [
        (
            "add_thread_dependency",
            json!({"thread_id": parent, "depends_on_thread_id": dep}),
        ),
        (
            "transition_thread",
            json!({"thread_id": dep, "action": "start_review"}),
        ),
        (
            "transition_thread",
            json!({"thread_id": dep, "action": "close"}),
        ),
    ] {
        let resp = mcp_call(client, base, token, tool, args).await;
        assert!(resp.get("error").is_none(), "{tool}: {resp}");
    }
}

/// `wait_for_ready` with no channel pins no thread: the workspace id is the
/// only filter that can stop another workspace's `ThreadReady` from waking the
/// waiter. A thread-pinned wait (like `wait_for_result` above) would pass even
/// with workspace filtering broken, because the thread ids already differ.
#[tokio::test]
async fn http_workspace_scoped_wait_ignores_other_workspaces_ready() {
    let (h, _bus) = spawn_sqlite().await;
    let base = format!("http://{}", h.addr);
    let client = client();
    let (token_a, parent_a, dep_a) = workspace_with_dep_pair(h.store.as_ref(), &base, "ws-a").await;
    let (token_b, parent_b, dep_b) = workspace_with_dep_pair(h.store.as_ref(), &base, "ws-b").await;

    // Phase 1: A's waiter must not wake when B's dependency closes.
    let wait_client = client.clone();
    let wait_base = base.clone();
    let token_a_c = token_a.clone();
    let waiter = tokio::spawn(async move {
        mcp_call(
            &wait_client,
            &wait_base,
            &token_a_c,
            "wait_for_ready",
            json!({"timeout_ms": 1500}),
        )
        .await
    });
    tokio::time::sleep(Duration::from_millis(500)).await;
    close_dependency(&client, &base, &token_b, &parent_b, &dep_b).await;

    let wait_resp = tokio::time::timeout(Duration::from_secs(15), waiter)
        .await
        .expect("waiter finishes")
        .expect("waiter joins");
    assert!(wait_resp.get("error").is_none(), "{wait_resp}");
    assert_eq!(
        tool_text(&wait_resp),
        Value::Null,
        "workspace-scoped wait in A must not wake on B's ThreadReady"
    );

    // Phase 2: the same wait must wake when A's own dependency closes, so the
    // negative phase cannot pass vacuously on a broken wait.
    let wait_client = client.clone();
    let wait_base = base.clone();
    let token_a_c = token_a.clone();
    let waiter = tokio::spawn(async move {
        mcp_call(
            &wait_client,
            &wait_base,
            &token_a_c,
            "wait_for_ready",
            json!({"timeout_ms": 5000}),
        )
        .await
    });
    tokio::time::sleep(Duration::from_millis(500)).await;
    close_dependency(&client, &base, &token_a, &parent_a, &dep_a).await;

    let wait_resp = tokio::time::timeout(Duration::from_secs(15), waiter)
        .await
        .expect("waiter finishes")
        .expect("waiter joins");
    assert!(wait_resp.get("error").is_none(), "{wait_resp}");
    let woke = tool_text(&wait_resp);
    assert!(
        woke.is_object(),
        "workspace-scoped wait in A must wake on A's ThreadReady, got {woke}"
    );
    assert_eq!(woke["kind"].as_str().unwrap(), "thread_ready");
    assert_eq!(
        woke["thread_id"].as_str().unwrap(),
        parent_a,
        "the wake is for A's parent thread"
    );
}

//! RED-008: one tenant's MCP long-polls cannot take the instance from the
//! others.
//!
//! - A `wait_for_*` call gives back its in-flight permit once it parks, so
//!   waits parked by many workspaces never fill the ceiling every tenant
//!   shares, and another workspace's ordinary request is still served.
//! - One workspace may park at most 32 waits; the next is refused at once with
//!   the typed rate-limit error, while another workspace's wait still parks.
//! - A wait gives its permit back only once it is admitted. One refused for a
//!   missing capability, another workspace's thread, or a full workspace
//!   keeps its permit until it answers; an auth-disabled caller keeps it too.

use std::{
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};

use maidan_artifacts::LocalFsStore;
use maidan_auth::{expand_set, hash_secret, TokenSecret, AGENT_WORKER};
use maidan_bus::InMemoryBus;
use maidan_mcp::parked_waits::MAX_PARKED_WAITS_PER_WORKSPACE;
use maidan_server::{load_shed::RequestLimit, router, AppState};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::{
    MemberKind, NewApiToken, NewChannel, NewMember, NewThread, NewWorkspace, ThreadId, WorkspaceId,
};
use reqwest::StatusCode;
use serde_json::{json, Value};

/// The JSON-RPC code of a refused, retry-later call.
const RATE_LIMITED: i64 = -32029;

struct Harness {
    base: String,
    state: AppState,
    client: reqwest::Client,
    _dir: tempfile::TempDir,
}

async fn spawn(request_limit: Option<usize>) -> Harness {
    let pool = maidan_search::sqlite_pool_options()
        .max_connections(8)
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
    let mut state = AppState::for_tests(
        store,
        Arc::new(LocalFsStore::new(dir.path())),
        Arc::new(InMemoryBus::with_capacity(256)),
        search,
    );
    state.auth_disabled = false;
    state.test_identity_header = false;
    if let Some(max) = request_limit {
        state.request_limit = RequestLimit::new(max);
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let app = router(state.clone());
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Harness {
        base: format!("http://{addr}"),
        state,
        // A fresh connection per request, so no wait queues behind another on
        // a pooled connection.
        client: reqwest::Client::builder()
            .pool_max_idle_per_host(0)
            .timeout(Duration::from_secs(30))
            .build()
            .unwrap(),
        _dir: dir,
    }
}

/// A workspace with one worker token and a thread it may wait on.
struct Tenant {
    workspace: WorkspaceId,
    token: String,
    thread: ThreadId,
}

async fn tenant(store: &dyn Store, name: &str) -> Tenant {
    let ws = store
        .create_workspace(NewWorkspace { name: name.into() })
        .await
        .unwrap();
    let member = store
        .create_member(NewMember {
            workspace_id: ws.id,
            handle: format!("{name}-agent"),
            display_name: None,
            kind: MemberKind::Agent,
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
    let thread = store
        .create_thread(NewThread {
            channel_id: channel.id,
            parent_thread_id: None,
            title: Some("task".into()),
            description: None,
            created_by: None,
        })
        .await
        .unwrap();
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: ws.id,
            member_id: member.id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: expand_set(AGENT_WORKER).unwrap(),
            expires_at: None,
        })
        .await
        .unwrap();
    Tenant {
        workspace: ws.id,
        token: secret.as_str().to_string(),
        thread: thread.id,
    }
}

/// Start a `wait_for_result` that times out after `timeout_ms`; resolves to
/// the HTTP status and the JSON-RPC body.
fn wait(
    h: &Harness,
    t: &Tenant,
    id: usize,
    timeout_ms: u64,
) -> tokio::task::JoinHandle<(StatusCode, Value)> {
    let request = h
        .client
        .post(format!("{}/mcp", h.base))
        .bearer_auth(&t.token)
        .header("accept", "application/json, text/event-stream")
        .json(&json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": {
                "name": "wait_for_result",
                "arguments": {"thread_id": t.thread.0, "timeout_ms": timeout_ms},
            },
        }));
    tokio::spawn(async move {
        let resp = request.send().await.unwrap();
        let status = resp.status();
        (status, resp.json().await.unwrap_or(Value::Null))
    })
}

fn rate_limited(body: &Value) -> bool {
    body["error"]["code"].as_i64() == Some(RATE_LIMITED)
}

/// Poll until `done` holds or `within` passes; returns whether it held.
async fn eventually(within: Duration, mut done: impl FnMut() -> bool) -> bool {
    let deadline = tokio::time::Instant::now() + within;
    while tokio::time::Instant::now() < deadline {
        if done() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    done()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn waits_parked_by_several_workspaces_do_not_hold_the_permits_another_workspace_needs() {
    // Fewer permits than parked waits: if a parked wait kept its permit, the
    // waits would hold every one and shed the rest.
    let ceiling = 20;
    let h = spawn(Some(ceiling)).await;
    let store = h.state.store.clone();
    let victim = tenant(store.as_ref(), "victim").await;
    let mut holders = Vec::new();
    for i in 0..3 {
        holders.push(tenant(store.as_ref(), &format!("holder-{i}")).await);
    }

    let parked = || {
        holders
            .iter()
            .map(|t| h.state.mcp.parked_waits(t.workspace))
            .sum::<usize>()
    };
    // One at a time, each parked before the next is sent, so no request is
    // shed for arriving alongside the others: only a parked wait that kept
    // its permit can fill the ceiling.
    let per_tenant = 10;
    let mut waits = Vec::new();
    for i in 0..per_tenant {
        for t in &holders {
            waits.push(wait(&h, t, i, 8_000));
            let expected = waits.len();
            let parked_now = eventually(Duration::from_secs(5), || parked() == expected).await;
            assert!(
                parked_now,
                "wait {expected} did not park: the ceiling of {ceiling} is held by {} parked waits",
                parked()
            );
        }
    }
    // A wait takes its place under the cap before its remaining checks pass
    // and only then gives its permit back, so the last wait can count as
    // parked a moment before its permit returns. A wait that kept its permit
    // would hold it for its whole 8s timeout.
    let released = eventually(Duration::from_secs(2), || {
        h.state.request_limit.in_flight() == 0
    })
    .await;
    assert!(
        released,
        "a parked wait still holds an in-flight permit: {} in flight",
        h.state.request_limit.in_flight()
    );

    for _ in 0..5 {
        let resp = h
            .client
            .get(format!("{}/workspaces/{}", h.base, victim.workspace.0))
            .bearer_auth(&victim.token)
            .send()
            .await
            .unwrap();
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "another workspace was shed while waits were parked"
        );
    }

    for handle in waits {
        let (status, body) = handle.await.unwrap();
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(
            body.get("result").is_some(),
            "the wait timed out normally: {body}"
        );
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_workspace_past_its_wait_cap_is_refused_while_another_workspace_still_waits() {
    let h = spawn(None).await;
    let store = h.state.store.clone();
    let busy = tenant(store.as_ref(), "busy").await;
    let other = tenant(store.as_ref(), "other").await;

    let over = 8;
    let sent = MAX_PARKED_WAITS_PER_WORKSPACE + over;
    let mut waits = Vec::new();
    for i in 0..sent {
        waits.push(wait(&h, &busy, i, 5_000));
    }
    let at_cap = eventually(Duration::from_secs(5), || {
        h.state.mcp.parked_waits(busy.workspace) == MAX_PARKED_WAITS_PER_WORKSPACE
    })
    .await;
    assert!(
        at_cap,
        "busy parked {} waits, not its cap of {MAX_PARKED_WAITS_PER_WORKSPACE}",
        h.state.mcp.parked_waits(busy.workspace)
    );

    // The other workspace is not held to the busy one's cap.
    let other_wait = wait(&h, &other, 0, 500);
    let (status, body) = other_wait.await.unwrap();
    assert_eq!(status, StatusCode::OK);
    assert!(
        !rate_limited(&body) && body.get("result").is_some(),
        "another workspace's wait was refused: {body}"
    );

    let mut refused = 0;
    let mut served = 0;
    for handle in waits {
        let (status, body) = handle.await.unwrap();
        assert_eq!(status, StatusCode::OK, "{body}");
        if rate_limited(&body) {
            assert_eq!(body["error"]["data"]["retry_after_ms"], 1_000, "{body}");
            refused += 1;
        } else {
            assert!(body.get("result").is_some(), "{body}");
            served += 1;
        }
    }
    assert_eq!(
        (served, refused),
        (MAX_PARKED_WAITS_PER_WORKSPACE, over),
        "the busy workspace parks up to its cap and no further"
    );
    assert_eq!(
        h.state.mcp.parked_waits(busy.workspace),
        0,
        "a finished wait gives its place back"
    );
}

/// A `wait_for_result` request for `thread`, timing out after `timeout_ms`.
fn wait_request(thread: ThreadId, timeout_ms: u64) -> maidan_mcp::JsonRpcRequest {
    serde_json::from_value(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "wait_for_result",
            "arguments": {"thread_id": thread.0, "timeout_ms": timeout_ms},
        },
    }))
    .unwrap()
}

/// Serve `request` as `auth` the way the HTTP layer does, with a permit to
/// give back, and count how often it was given back.
async fn releases(
    h: &Harness,
    auth: &maidan_auth::AuthContext,
    request: maidan_mcp::JsonRpcRequest,
) -> (usize, Value) {
    let released = Arc::new(AtomicUsize::new(0));
    let counter = released.clone();
    let response = maidan_mcp::parked_waits::with_permit_release(
        move || {
            counter.fetch_add(1, Ordering::SeqCst);
        },
        h.state.mcp.handle(request, auth),
    )
    .await;
    (
        released.load(Ordering::SeqCst),
        serde_json::to_value(&response).unwrap(),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_wait_gives_back_its_permit_only_once_it_is_admitted() {
    let h = spawn(None).await;
    let store = h.state.store.clone();
    let mine = tenant(store.as_ref(), "mine").await;
    let theirs = tenant(store.as_ref(), "theirs").await;
    let auth = maidan_auth::resolve_bearer(store.as_ref(), &mine.token)
        .await
        .unwrap();

    // Admitted: given back exactly once.
    let (released, body) = releases(&h, &auth, wait_request(mine.thread, 50)).await;
    assert!(body.get("result").is_some(), "{body}");
    assert_eq!(released, 1, "an admitted wait gives its permit back once");

    // A token without the tool's capability.
    let member = store
        .create_member(NewMember {
            workspace_id: mine.workspace,
            handle: "poster".into(),
            display_name: None,
            kind: MemberKind::Agent,
        })
        .await
        .unwrap();
    let secret = TokenSecret::generate();
    store
        .create_api_token(NewApiToken {
            workspace_id: mine.workspace,
            member_id: member.id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: None,
            capabilities: vec!["message:post".into()],
            expires_at: None,
        })
        .await
        .unwrap();
    let poster = maidan_auth::resolve_bearer(store.as_ref(), secret.as_str())
        .await
        .unwrap();
    let (released, body) = releases(&h, &poster, wait_request(mine.thread, 50)).await;
    assert!(body.get("error").is_some(), "{body}");
    assert_eq!(
        released, 0,
        "a wait refused for its capability kept its permit"
    );

    // Another workspace's thread.
    let (released, body) = releases(&h, &auth, wait_request(theirs.thread, 50)).await;
    assert!(body.get("error").is_some(), "{body}");
    assert_eq!(
        released, 0,
        "a wait on another tenant's thread kept its permit"
    );

    // A workspace already at its cap.
    let mut parked = Vec::new();
    for i in 0..MAX_PARKED_WAITS_PER_WORKSPACE {
        parked.push(wait(&h, &mine, i, 5_000));
    }
    let full = eventually(Duration::from_secs(5), || {
        h.state.mcp.parked_waits(mine.workspace) == MAX_PARKED_WAITS_PER_WORKSPACE
    })
    .await;
    assert!(full, "the workspace did not reach its cap");
    let (released, body) = releases(&h, &auth, wait_request(mine.thread, 50)).await;
    assert!(rate_limited(&body), "{body}");
    assert_eq!(released, 0, "a wait refused at the cap kept its permit");

    // An auth-disabled caller holds no parking place, so it keeps its permit.
    let (released, _) = releases(
        &h,
        &maidan_auth::AuthContext::bypass(),
        wait_request(mine.thread, 50),
    )
    .await;
    assert_eq!(released, 0, "an auth-disabled wait gave back its permit");

    for handle in parked {
        handle.await.unwrap();
    }
}

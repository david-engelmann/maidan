//! The AG-UI SSE door (`GET /agui/stream`).
//!
//! Proves the door end-to-end: a thread's lifecycle (create → post a message)
//! reaches an AG-UI client as `RUN_STARTED` then `TEXT_MESSAGE_*` frames, each
//! carrying its source event-log `id:`, and a reconnect with `Last-Event-ID`
//! resumes past what was already seen. The caller is the workspace's admin,
//! who can see every event, so the mapping + SSE framing + resume are
//! exercised; the per-event RBAC filter composes the separately-tested
//! `can_access_*` helpers.

use std::{net::SocketAddr, sync::Arc, time::Duration};

use futures::StreamExt;
use maidan_artifacts::LocalFsStore;
use maidan_server::{
    router,
    test_support::{self, Tenant},
    AppState,
};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::MemberKind;
use serde_json::{json, Value};

struct Harness {
    addr: SocketAddr,
    tenant: Tenant,
    store: Arc<dyn Store>,
    server: tokio::task::JoinHandle<()>,
    _dir: tempfile::TempDir,
}

/// A client sending `token` with no timeout: the SSE response stays open.
fn streaming_client(token: &str) -> reqwest::Client {
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::AUTHORIZATION,
        test_support::bearer(token).parse().unwrap(),
    );
    reqwest::Client::builder()
        .default_headers(headers)
        .build()
        .unwrap()
}

async fn spawn(workspace: &str) -> Harness {
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
    let bus = Arc::new(maidan_bus::InMemoryBus::with_capacity(256));
    let tenant = test_support::tenant(store.as_ref(), workspace).await;
    let app = router(AppState::for_tests(store.clone(), artifacts, bus, search));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Harness {
        addr,
        tenant,
        store,
        server,
        _dir: dir,
    }
}

/// An AG-UI SSE frame: its `id:` (source event-log id) and parsed `data:` JSON.
struct Frame {
    id: Option<String>,
    data: Value,
}

/// Pull AG-UI frames from a streaming SSE response until `want` are collected.
async fn collect_frames(resp: reqwest::Response, want: usize) -> Vec<Frame> {
    let mut stream = resp.bytes_stream();
    let mut buf = String::new();
    let mut frames = Vec::new();
    let work = async {
        while frames.len() < want {
            let Some(chunk) = stream.next().await else {
                break;
            };
            buf.push_str(&String::from_utf8_lossy(&chunk.unwrap()));
            while let Some(idx) = buf.find("\n\n") {
                let raw: String = buf.drain(..idx + 2).collect();
                let mut id = None;
                let mut data = None;
                for line in raw.lines() {
                    if let Some(v) = line.strip_prefix("id:") {
                        id = Some(v.trim().to_string());
                    } else if let Some(v) = line.strip_prefix("data:") {
                        data = serde_json::from_str::<Value>(v.trim()).ok();
                    }
                }
                if let Some(data) = data {
                    frames.push(Frame { id, data });
                }
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(5), work)
        .await
        .expect("timeout collecting AG-UI frames");
    frames
}

fn types(frames: &[Frame]) -> Vec<String> {
    frames
        .iter()
        .map(|f| f.data["type"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn agui_stream_maps_thread_lifecycle_to_run_events() {
    let h = spawn("agui").await;
    let client = streaming_client(&h.tenant.token);
    let base = format!("http://{}", h.addr);
    let post = |path: String, body: Value| {
        let client = client.clone();
        async move {
            client
                .post(path)
                .json(&body)
                .send()
                .await
                .unwrap()
                .json::<Value>()
                .await
                .unwrap()
        }
    };

    let workspace_id = h.tenant.workspace_id.0;
    let actor = h
        .tenant
        .member(h.store.as_ref(), "agent", MemberKind::Human)
        .await;
    let ch = post(
        format!("{base}/workspaces/{workspace_id}/channels"),
        json!({"name": "runs"}),
    )
    .await;
    let channel_id = ch["id"].as_str().unwrap().to_string();

    // Subscribe live to the workspace's AG-UI stream, then drive a run.
    let resp = client
        .get(format!("{base}/agui/stream?workspace_id={workspace_id}"))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success(), "sse status {}", resp.status());
    tokio::time::sleep(Duration::from_millis(150)).await;

    let thread = post(
        format!("{base}/channels/{channel_id}/threads"),
        json!({"title": "do the thing"}),
    )
    .await;
    let thread_id = thread["id"].as_str().unwrap().to_string();
    let _ = actor
        .client()
        .post(format!("{base}/threads/{thread_id}/messages"))
        .json(&json!({"body": "on it"}))
        .send()
        .await
        .unwrap()
        .json::<Value>()
        .await
        .unwrap();

    // ThreadCreated → RUN_STARTED; MessagePosted → TEXT_MESSAGE_START/CONTENT/END.
    let frames = collect_frames(resp, 4).await;
    assert_eq!(
        types(&frames),
        vec![
            "RUN_STARTED",
            "TEXT_MESSAGE_START",
            "TEXT_MESSAGE_CONTENT",
            "TEXT_MESSAGE_END",
        ]
    );
    // RUN_STARTED's runId is the thread id (a thread is a run).
    assert_eq!(
        frames[0].data["threadId"].as_str(),
        Some(thread_id.as_str())
    );
    assert_eq!(frames[0].data["runId"].as_str(), Some(thread_id.as_str()));
    assert_eq!(frames[2].data["delta"], "on it");
    // Every frame carries its source event-log id (the resume anchor).
    assert!(frames.iter().all(|f| f.id.is_some()), "frames carry an id:");

    h.server.abort();
}

#[tokio::test]
async fn agui_stream_resumes_after_a_last_event_id() {
    let h = spawn("resume").await;
    let client = streaming_client(&h.tenant.token);
    let base = format!("http://{}", h.addr);
    let post = |path: String, body: Value| {
        let client = client.clone();
        async move {
            client
                .post(path)
                .json(&body)
                .send()
                .await
                .unwrap()
                .json::<Value>()
                .await
                .unwrap()
        }
    };

    let workspace_id = h.tenant.workspace_id.0;
    let ch = post(
        format!("{base}/workspaces/{workspace_id}/channels"),
        json!({"name": "c"}),
    )
    .await;
    let channel_id = ch["id"].as_str().unwrap().to_string();

    // Watch live, then drive two runs and capture the first run's frame id.
    let resp = client
        .get(format!("{base}/agui/stream?workspace_id={workspace_id}"))
        .send()
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(150)).await;
    let first = post(
        format!("{base}/channels/{channel_id}/threads"),
        json!({"title": "run one"}),
    )
    .await;
    let second = post(
        format!("{base}/channels/{channel_id}/threads"),
        json!({"title": "run two"}),
    )
    .await;
    let frames = collect_frames(resp, 2).await;
    assert_eq!(types(&frames), vec!["RUN_STARTED", "RUN_STARTED"]);
    assert_eq!(
        frames[0].data["threadId"].as_str(),
        first["id"].as_str(),
        "first live RUN_STARTED is run one"
    );
    let after = frames[0].id.clone().unwrap();

    // Reconnect with Last-Event-ID = the first run's id → only the second run replays.
    let resp2 = client
        .get(format!("{base}/agui/stream?workspace_id={workspace_id}"))
        .header("Last-Event-ID", &after)
        .send()
        .await
        .unwrap();
    let frames2 = collect_frames(resp2, 1).await;
    assert_eq!(types(&frames2), vec!["RUN_STARTED"]);
    assert_eq!(
        frames2[0].data["threadId"].as_str(),
        second["id"].as_str(),
        "resume replays only the second run, skipping the already-seen first"
    );

    h.server.abort();
}

/// A resume that names no workspace replays the caller's own: a token is
/// bound to one workspace, so an omitted `workspace_id` means that one, never
/// "every workspace". Another tenant's run, logged first, must not replay.
#[tokio::test]
async fn agui_stream_resume_without_a_workspace_replays_only_the_callers() {
    let h = spawn("own").await;
    let base = format!("http://{}", h.addr);
    let post = |token: String, path: String, body: Value| async move {
        let resp = streaming_client(&token)
            .post(path)
            .json(&body)
            .send()
            .await
            .unwrap();
        assert!(resp.status().is_success(), "POST status {}", resp.status());
        resp.json::<Value>().await.unwrap()
    };
    let start_run = |token: String, workspace_id: uuid::Uuid, title: &'static str| {
        let base = base.clone();
        async move {
            let ch = post(
                token.clone(),
                format!("{base}/workspaces/{workspace_id}/channels"),
                json!({"name": "runs"}),
            )
            .await;
            let channel_id = ch["id"].as_str().unwrap().to_string();
            post(
                token,
                format!("{base}/channels/{channel_id}/threads"),
                json!({"title": title}),
            )
            .await
        }
    };

    let other = test_support::tenant(h.store.as_ref(), "other").await;
    let foreign = start_run(other.token.clone(), other.workspace_id.0, "theirs").await;
    let own = start_run(h.tenant.token.clone(), h.tenant.workspace_id.0, "ours").await;

    // Resume from the caller's first event, naming no workspace.
    let from = h
        .store
        .min_event_id(h.tenant.workspace_id)
        .await
        .unwrap()
        .unwrap();
    let resp = streaming_client(&h.tenant.token)
        .get(format!("{base}/agui/stream?after_id={from}"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);
    let frames = collect_frames(resp, 1).await;
    assert_eq!(types(&frames), vec!["RUN_STARTED"]);
    assert_eq!(
        frames[0].data["threadId"].as_str(),
        own["id"].as_str(),
        "the replay is the caller's run"
    );
    assert_ne!(
        frames[0].data["threadId"].as_str(),
        foreign["id"].as_str(),
        "another tenant's run never replays"
    );

    h.server.abort();
}

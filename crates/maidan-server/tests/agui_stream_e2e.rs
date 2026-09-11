//! The AG-UI SSE door (`GET /agui/stream`, Cluster 369.2, Wave 2 #17, H1).
//!
//! Proves the door end-to-end: a thread's lifecycle (create → post a message)
//! reaches an AG-UI client as `RUN_STARTED` then `TEXT_MESSAGE_*` frames, each
//! carrying its source event-log `id:`, and a reconnect with `Last-Event-ID`
//! resumes past what was already seen. Auth is bypass here (like the other
//! `/mcp/stream` e2es), so the mapping + SSE framing + resume are exercised; the
//! per-event RBAC filter composes the separately-tested `can_access_*` helpers.

use std::{net::SocketAddr, sync::Arc, time::Duration};

use futures::StreamExt;
use maidan_artifacts::LocalFsStore;
use maidan_server::{router, AppState};
use maidan_store::{prelude::*, run_sqlite_migrations};
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

async fn spawn() -> (
    SocketAddr,
    reqwest::Client,
    tokio::task::JoinHandle<()>,
    tempfile::TempDir,
) {
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
    let bus = Arc::new(maidan_bus::InMemoryBus::with_capacity(256));
    let app = router(AppState::for_tests(store, artifacts, bus, search));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    // No client timeout: the SSE response stays open.
    (addr, reqwest::Client::new(), server, dir)
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
    let (addr, client, server, _dir) = spawn().await;
    let base = format!("http://{addr}");
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

    let ws = post(format!("{base}/workspaces"), json!({"name": "agui"})).await;
    let workspace_id = ws["id"].as_str().unwrap().to_string();
    let actor = post(
        format!("{base}/workspaces/{workspace_id}/members"),
        json!({"handle": "agent", "kind": "human"}),
    )
    .await;
    let actor_id = actor["id"].as_str().unwrap().to_string();
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
    let _ = post(
        format!("{base}/threads/{thread_id}/messages"),
        json!({"author_id": actor_id, "body": "on it"}),
    )
    .await;

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

    server.abort();
}

#[tokio::test]
async fn agui_stream_resumes_after_a_last_event_id() {
    let (addr, client, server, _dir) = spawn().await;
    let base = format!("http://{addr}");
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

    let ws = post(format!("{base}/workspaces"), json!({"name": "resume"})).await;
    let workspace_id = ws["id"].as_str().unwrap().to_string();
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

    server.abort();
}

#[tokio::test]
async fn agui_stream_rejects_after_id_without_workspace() {
    let (addr, client, server, _dir) = spawn().await;
    let resp = client
        .get(format!("http://{addr}/agui/stream?after_id=5"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::BAD_REQUEST);
    server.abort();
}

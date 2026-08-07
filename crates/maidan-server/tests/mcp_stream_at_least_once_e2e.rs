//! MCP SSE (`GET /mcp/stream`) at-least-once parity (Cluster 126).
//!
//! Proves the SSE wiring routes an `at_least_once` (`workspace_id + consumer_id`)
//! stream through the reconcile loop: the stable backlog is delivered in order
//! and the durable delivery cursor advances. The cross-reconnect floor / no-
//! re-delivery property is shared `reconcile_deliver` logic, covered
//! deterministically by the WebSocket e2e (`ws_subscribe_e2e`).

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
    time::Duration,
};

use futures::StreamExt;
use maidan_artifacts::LocalFsStore;
use maidan_server::{router, subscribe_resume, AppState, FederationRuntime};
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

async fn spawn() -> (
    SocketAddr,
    reqwest::Client,
    Arc<dyn Store>,
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
    let mut state = AppState::new(
        store.clone(),
        artifacts,
        bus,
        search,
        Arc::new(maidan_search::HashV1Provider),
        true,
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    state.subscribe_resume_secret = Some(Arc::from(subscribe_resume::TEST_SUBSCRIBE_RESUME_SECRET));
    // Deterministic, fast reconcile via AppState config (no process-wide env).
    state.delivery_stability = Duration::ZERO;
    state.delivery_reconcile_interval = Duration::from_millis(150);
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    // No client-level timeout: the SSE response stays open.
    let client = reqwest::Client::new();
    (addr, client, store, server, dir)
}

/// Pull SSE `data:` payloads from a streaming response until `want` non-control
/// event kinds are collected, capturing the `subscribe_ack` `after_id` too.
async fn collect_events(resp: reqwest::Response, want: usize) -> (Vec<String>, Option<i64>) {
    let mut stream = resp.bytes_stream();
    let mut buf = String::new();
    let mut kinds = Vec::new();
    let mut ack_after = None;
    let work = async {
        while kinds.len() < want {
            let Some(chunk) = stream.next().await else {
                break;
            };
            buf.push_str(&String::from_utf8_lossy(&chunk.unwrap()));
            while let Some(idx) = buf.find("\n\n") {
                let frame: String = buf.drain(..idx + 2).collect();
                for line in frame.lines() {
                    let Some(data) = line.strip_prefix("data:") else {
                        continue; // keep-alive comments / event: lines
                    };
                    let Ok(v) = serde_json::from_str::<Value>(data.trim()) else {
                        continue;
                    };
                    match v.get("type").and_then(|t| t.as_str()) {
                        Some("subscribe_ack") => ack_after = v["after_id"].as_i64(),
                        Some(_) => {} // other control frames
                        None => kinds.push(v["kind"].as_str().unwrap().to_string()),
                    }
                }
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(5), work)
        .await
        .expect("timeout collecting SSE events");
    (kinds, ack_after)
}

#[tokio::test]
async fn mcp_stream_at_least_once_delivers_backlog_in_order_and_advances_cursor() {
    let (addr, client, store, server, _dir) = spawn().await;
    let base = format!("http://{addr}");

    // Backlog: workspace_created + two member_joined.
    let ws_resp: Value = client
        .post(format!("{base}/workspaces"))
        .json(&json!({"name": "mcp-alo"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let workspace_id = ws_resp["id"].as_str().unwrap().to_string();
    for body in [
        json!({"handle": "alice", "kind": "human"}),
        json!({"handle": "_chan", "kind": "human"}),
    ] {
        let _: Value = client
            .post(format!("{base}/workspaces/{workspace_id}/members"))
            .json(&body)
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
    }

    let url = format!(
        "{base}/mcp/stream?workspace_id={workspace_id}&consumer_id=mcp-alo&at_least_once=true"
    );
    let resp = client.get(&url).send().await.unwrap();
    assert!(resp.status().is_success(), "sse status {}", resp.status());
    let (kinds, ack_after) = collect_events(resp, 3).await;
    assert_eq!(ack_after, Some(0), "fresh consumer starts at after_id 0");
    assert_eq!(
        kinds,
        vec!["workspace_created", "member_joined", "member_joined"]
    );

    // The reconcile loop advances the durable cursor past the delivered backlog.
    let ws_id = maidan_types::WorkspaceId(uuid::Uuid::parse_str(&workspace_id).unwrap());
    let wait_cursor = async {
        loop {
            if store.get_delivery_cursor("mcp-alo", ws_id).await.unwrap() >= 3 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
    };
    tokio::time::timeout(Duration::from_secs(5), wait_cursor)
        .await
        .expect("delivery cursor did not advance to 3");

    server.abort();
}

#[tokio::test]
async fn mcp_stream_filters_by_event_kind() {
    let (addr, client, _store, server, _dir) = spawn().await;
    let base = format!("http://{addr}");

    let ws: Value = client
        .post(format!("{base}/workspaces"))
        .json(&json!({"name": "filt"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let workspace_id = ws["id"].as_str().unwrap().to_string();

    // Live subscription narrowed to channel_created only (Cluster 150).
    let resp = client
        .get(format!(
            "{base}/mcp/stream?workspace_id={workspace_id}&kinds=channel_created"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::OK);

    // Let the subscription attach, then fire a member_joined (excluded) followed
    // by a channel_created (included).
    tokio::time::sleep(Duration::from_millis(150)).await;
    let _: Value = client
        .post(format!("{base}/workspaces/{workspace_id}/members"))
        .json(&json!({"handle": "bob", "kind": "human"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let _: Value = client
        .post(format!("{base}/workspaces/{workspace_id}/channels"))
        .json(&json!({"name": "general"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();

    // The first delivered event is the channel_created — member_joined was filtered.
    let (kinds, _ack) = collect_events(resp, 1).await;
    assert_eq!(
        kinds[0], "channel_created",
        "the kinds filter must exclude member_joined"
    );

    server.abort();
}

#[tokio::test]
async fn mcp_stream_rejects_unknown_kind() {
    let (addr, client, _store, server, _dir) = spawn().await;
    let base = format!("http://{addr}");
    let ws: Value = client
        .post(format!("{base}/workspaces"))
        .json(&json!({"name": "bad"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let workspace_id = ws["id"].as_str().unwrap().to_string();
    let resp = client
        .get(format!(
            "{base}/mcp/stream?workspace_id={workspace_id}&kinds=not_a_real_kind"
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), reqwest::StatusCode::BAD_REQUEST);
    server.abort();
}

/// Read the first domain-event frame (skips control frames like subscribe_ack).
async fn first_event_frame(resp: reqwest::Response) -> Value {
    let mut stream = resp.bytes_stream();
    let mut buf = String::new();
    let work = async {
        loop {
            let Some(chunk) = stream.next().await else {
                panic!("stream ended before an event frame");
            };
            buf.push_str(&String::from_utf8_lossy(&chunk.unwrap()));
            while let Some(idx) = buf.find("\n\n") {
                let frame: String = buf.drain(..idx + 2).collect();
                for line in frame.lines() {
                    let Some(data) = line.strip_prefix("data:") else {
                        continue;
                    };
                    let Ok(v) = serde_json::from_str::<Value>(data.trim()) else {
                        continue;
                    };
                    if v.get("type").is_none() {
                        return v; // a domain-event frame (control frames carry "type")
                    }
                }
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(5), work)
        .await
        .expect("timeout waiting for an event frame")
}

/// Cluster 178 (token round 3): `lean=true` delivers `{log_id, kind, ...ids}`
/// pointer frames — the heavy embedded event payload is dropped, but the
/// top-level routing fields clients read stay put.
#[tokio::test]
async fn mcp_stream_lean_frames_omit_the_event_payload() {
    let (addr, client, _store, server, _dir) = spawn().await;
    let base = format!("http://{addr}");
    let ws: Value = client
        .post(format!("{base}/workspaces"))
        .json(&json!({"name": "lean-ws"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let workspace_id = ws["id"].as_str().unwrap().to_string();

    // Backlog has workspace_created. Subscribe lean + at_least_once (deterministic
    // backlog delivery) and inspect the first event frame.
    let resp = client
        .get(format!(
            "{base}/mcp/stream?workspace_id={workspace_id}&consumer_id=lean&at_least_once=true&lean=true"
        ))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success());
    let frame = first_event_frame(resp).await;

    // Routing fields present…
    assert!(
        frame["log_id"].is_number(),
        "lean frame keeps log_id: {frame}"
    );
    assert_eq!(frame["kind"], "workspace_created");
    assert_eq!(frame["workspace_id"].as_str(), Some(workspace_id.as_str()));
    // …but the heavy embedded payload is gone (full frame would carry `workspace`).
    assert!(
        frame.get("workspace").is_none(),
        "lean frame must drop the embedded event payload: {frame}"
    );
    server.abort();
}

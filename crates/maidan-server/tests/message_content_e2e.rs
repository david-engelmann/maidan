//! Cluster 173: structured message content over REST.
//!
//! Proves: posting typed `content` derives the searchable `body`; GET returns
//! the blocks; a plain body-only post has `content: null`; editing content
//! re-derives body. Runs with auth bypassed (`AppState::for_tests`).

use std::{net::SocketAddr, sync::Arc};

use maidan_artifacts::LocalFsStore;
use maidan_server::{router, AppState};
use maidan_store::{prelude::*, run_sqlite_migrations};
use sqlx::sqlite::SqlitePoolOptions;

struct Ctx {
    addr: SocketAddr,
    _server: tokio::task::JoinHandle<()>,
    client: reqwest::Client,
    _dir: tempfile::TempDir,
}

impl Ctx {
    fn base(&self) -> String {
        format!("http://{}", self.addr)
    }
}

async fn spawn() -> Ctx {
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
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    let app = router(AppState::for_tests(store, artifacts, bus, search));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Ctx {
        addr,
        _server: server,
        client: reqwest::Client::new(),
        _dir: dir,
    }
}

/// Create ws → member → channel → thread, returning (base, thread_id, member_id).
async fn seed(ctx: &Ctx) -> (String, String, String) {
    let base = ctx.base();
    let c = &ctx.client;
    let ws: serde_json::Value = c
        .post(format!("{base}/workspaces"))
        .json(&serde_json::json!({"name": "acme"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let wid = ws["id"].as_str().unwrap().to_string();
    let mem: serde_json::Value = c
        .post(format!("{base}/workspaces/{wid}/members"))
        .json(&serde_json::json!({"handle": "agent", "kind": "agent"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let mid = mem["id"].as_str().unwrap().to_string();
    let ch: serde_json::Value = c
        .post(format!("{base}/workspaces/{wid}/channels"))
        .json(&serde_json::json!({"name": "general"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let cid = ch["id"].as_str().unwrap().to_string();
    let th: serde_json::Value = c
        .post(format!("{base}/channels/{cid}/threads"))
        .json(&serde_json::json!({"title": "t"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let tid = th["id"].as_str().unwrap().to_string();
    (base, tid, mid)
}

#[tokio::test]
async fn content_post_derives_body_and_round_trips() {
    let ctx = spawn().await;
    let (base, tid, mid) = seed(&ctx).await;
    let c = &ctx.client;

    // Post with typed content, no body → body is derived.
    let posted: serde_json::Value = c
        .post(format!("{base}/threads/{tid}/messages"))
        .json(&serde_json::json!({
            "author_id": mid,
            "content": [
                {"type": "text", "text": "deploying now"},
                {"type": "code", "language": "sh", "code": "cargo build"},
                {"type": "tool_use", "id": "t1", "name": "shell", "input": {"cmd": "ls"}}
            ]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    // body derived from text + fenced code; tool_use contributes nothing.
    let body = posted["body"].as_str().unwrap();
    assert!(body.contains("deploying now"), "body={body}");
    assert!(body.contains("```sh\ncargo build\n```"), "body={body}");
    assert!(
        !body.contains("shell"),
        "tool_use name must not leak into body"
    );
    assert_eq!(posted["content"].as_array().unwrap().len(), 3);
    assert_eq!(posted["content"][2]["type"], "tool_use");

    // GET the thread messages → content survives the round-trip.
    let msgs: serde_json::Value = c
        .get(format!("{base}/threads/{tid}/messages"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let got = &msgs.as_array().unwrap()[0];
    assert_eq!(got["content"].as_array().unwrap().len(), 3);
    assert_eq!(got["content"][1]["language"], "sh");
}

#[tokio::test]
async fn plain_body_post_has_null_content() {
    let ctx = spawn().await;
    let (base, tid, mid) = seed(&ctx).await;
    let posted: serde_json::Value = ctx
        .client
        .post(format!("{base}/threads/{tid}/messages"))
        .json(&serde_json::json!({"author_id": mid, "body": "just text"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(posted["body"], "just text");
    // content omitted (skip_serializing_if None) → absent/null.
    assert!(posted.get("content").is_none() || posted["content"].is_null());
}

#[tokio::test]
async fn editing_content_re_derives_body() {
    let ctx = spawn().await;
    let (base, tid, mid) = seed(&ctx).await;
    let c = &ctx.client;
    let posted: serde_json::Value = c
        .post(format!("{base}/threads/{tid}/messages"))
        .json(&serde_json::json!({"author_id": mid, "content": [{"type": "text", "text": "v1"}]}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let msg_id = posted["id"].as_str().unwrap();

    let edited: serde_json::Value = c
        .patch(format!("{base}/messages/{msg_id}"))
        .json(&serde_json::json!({
            "editor_id": mid,
            "content": [{"type": "text", "text": "v2 updated"}]
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(edited["body"], "v2 updated");
    assert_eq!(edited["content"][0]["text"], "v2 updated");
}

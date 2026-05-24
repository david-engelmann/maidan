//! End-to-end test for `GET /workspaces/:wid/search` and the MCP
//! `search_messages` tool. Both share `Arc<dyn Search>` in AppState.

use std::{sync::Arc, time::Duration};

use maidan_artifacts::LocalFsStore;
use maidan_bus::InMemoryBus;
use maidan_search::SqliteSearch;
use maidan_server::{router, AppState};
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use reqwest::StatusCode;
use serde_json::{json, Value};
use sqlx::sqlite::SqlitePoolOptions;

async fn spawn() -> (
    std::net::SocketAddr,
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
    let search: Arc<dyn maidan_search::Search> = Arc::new(SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(InMemoryBus::with_capacity(256));
    let app = router(AppState::for_tests(store, artifacts, bus, search));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    (addr, client, server, dir)
}

async fn seed_corpus(client: &reqwest::Client, base: &str) -> (String, String, String) {
    let ws: Value = client
        .post(format!("{base}/workspaces"))
        .json(&json!({"name": "search-ws"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let workspace_id = ws["id"].as_str().unwrap().to_string();

    let alice: Value = client
        .post(format!("{base}/workspaces/{workspace_id}/members"))
        .json(&json!({"handle": "alice", "kind": "human"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let alice_id = alice["id"].as_str().unwrap().to_string();

    let ch: Value = client
        .post(format!("{base}/workspaces/{workspace_id}/channels"))
        .json(&json!({"name": "general"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let channel_id = ch["id"].as_str().unwrap().to_string();

    let th: Value = client
        .post(format!("{base}/channels/{channel_id}/threads"))
        .json(&json!({"title": "search-thread"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let thread_id = th["id"].as_str().unwrap().to_string();

    for body in [
        "rust is a systems programming language",
        "tokio powers async rust applications",
        "the deployment shipped without rust",
        "rollback the deployment immediately",
    ] {
        let _: Value = client
            .post(format!("{base}/threads/{thread_id}/messages"))
            .json(&json!({"author_id": alice_id, "body": body}))
            .send()
            .await
            .unwrap()
            .json()
            .await
            .unwrap();
    }

    (workspace_id, channel_id, thread_id)
}

#[tokio::test]
async fn http_search_returns_ranked_hits() {
    let (addr, client, server, _dir) = spawn().await;
    let base = format!("http://{addr}");

    let (workspace_id, _, _) = seed_corpus(&client, &base).await;

    let hits: Vec<Value> = client
        .get(format!("{base}/workspaces/{workspace_id}/search?q=rust"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        hits.len(),
        3,
        "rust query should match 3 messages: {hits:?}"
    );
    for hit in &hits {
        let body = hit["body"].as_str().unwrap().to_lowercase();
        assert!(
            body.contains("rust"),
            "expected body to contain rust: {body}"
        );
    }

    // Unknown term returns empty array, not 404.
    let hits: Vec<Value> = client
        .get(format!("{base}/workspaces/{workspace_id}/search?q=xyzzy"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(hits.is_empty());

    // Empty query returns 400 problem+json.
    let resp = client
        .get(format!("{base}/workspaces/{workspace_id}/search?q=%20%20"))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        resp.headers().get("content-type").unwrap(),
        "application/problem+json"
    );

    server.abort();
}

#[tokio::test]
async fn mcp_search_messages_tool_works() {
    let (addr, client, server, _dir) = spawn().await;
    let base = format!("http://{addr}");
    let (workspace_id, _, _) = seed_corpus(&client, &base).await;

    let resp: Value = client
        .post(format!("{base}/mcp"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "tools/list",
            "params": {}
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let names: Vec<&str> = resp["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"search_messages"));

    let resp: Value = client
        .post(format!("{base}/mcp"))
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "tools/call",
            "params": {
                "name": "search_messages",
                "arguments": {
                    "workspace_id": workspace_id,
                    "query": "rust"
                }
            }
        }))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    let hits: Vec<Value> = serde_json::from_str(text).unwrap();
    assert_eq!(hits.len(), 3);
    for hit in &hits {
        assert!(hit["body"]
            .as_str()
            .unwrap()
            .to_lowercase()
            .contains("rust"));
    }

    server.abort();
}

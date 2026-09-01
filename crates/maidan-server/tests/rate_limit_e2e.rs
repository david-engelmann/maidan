//! Rate limit middleware returns 429 when MAIDAN_RATE_LIMIT_MAX is set.

use std::{net::SocketAddr, sync::Arc, time::Duration};

use maidan_artifacts::LocalFsStore;
use maidan_server::{router, AppState};
use maidan_store::{configure_sqlite_pool, prelude::*, run_sqlite_migrations};
use reqwest::StatusCode;
use sqlx::sqlite::SqlitePoolOptions;

async fn spawn() -> (SocketAddr, tokio::task::JoinHandle<()>, tempfile::TempDir) {
    unsafe {
        std::env::set_var("MAIDAN_RATE_LIMIT_MAX", "3");
        std::env::set_var("MAIDAN_RATE_LIMIT_WINDOW_SECS", "60");
    }

    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    configure_sqlite_pool(&pool).await.expect("pragmas");
    run_sqlite_migrations(&pool).await.expect("migrate");
    let store = Arc::new(SqliteStore::new(pool.clone()));
    let dir = tempfile::tempdir().expect("tempdir");
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let app = router(AppState::for_tests(store, artifacts, bus, search));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.expect("serve");
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    (addr, handle, dir)
}

#[tokio::test]
async fn health_endpoints_are_exempt_from_rate_limit() {
    let (addr, handle, _dir) = spawn().await;
    let base = format!("http://{addr}");
    let client = reqwest::Client::new();
    for _ in 0..5 {
        let resp = client
            .get(format!("{base}/health/live"))
            .send()
            .await
            .expect("request");
        assert_eq!(resp.status(), StatusCode::OK);
    }
    handle.abort();
}

#[tokio::test]
async fn burst_over_limit_returns_429_problem_json() {
    let (addr, handle, _dir) = spawn().await;
    let base = format!("http://{addr}");
    let client = reqwest::Client::builder()
        .default_headers({
            let mut h = reqwest::header::HeaderMap::new();
            h.insert("x-forwarded-for", "10.0.0.2".parse().unwrap());
            h
        })
        .build()
        .expect("client");
    let ws = client
        .post(format!("{base}/workspaces"))
        .json(&serde_json::json!({"name": "rl-ws"}))
        .send()
        .await
        .expect("create ws")
        .json::<serde_json::Value>()
        .await
        .expect("json");
    let wid = ws["id"].as_str().unwrap();

    for _ in 0..2 {
        let resp = client
            .get(format!("{base}/workspaces/{wid}"))
            .send()
            .await
            .expect("get");
        assert_eq!(resp.status(), StatusCode::OK);
    }
    let resp = client
        .get(format!("{base}/workspaces/{wid}"))
        .send()
        .await
        .expect("limited");
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(resp.headers().get("retry-after").is_some());
    let body: serde_json::Value = resp.json().await.expect("problem");
    assert_eq!(body["status"], 429);
    assert!(body["type"].as_str().unwrap().contains("rate-limited"));
    handle.abort();
}

#[tokio::test]
async fn mcp_rate_limit_returns_jsonrpc_backpressure_envelope() {
    // Cluster 172: a rate-limited POST /mcp must return a JSON-RPC error
    // envelope (code -32029 + data.retry_after_ms), not the plain problem+json,
    // so an agent's JSON-RPC layer gets a typed backpressure signal.
    let (addr, handle, _dir) = spawn().await;
    let base = format!("http://{addr}");
    let client = reqwest::Client::builder()
        .default_headers({
            let mut h = reqwest::header::HeaderMap::new();
            // Distinct client key so this test's bucket is isolated.
            h.insert("x-forwarded-for", "10.0.0.42".parse().unwrap());
            h.insert("MCP-Protocol-Version", "2024-11-05".parse().unwrap());
            h
        })
        .build()
        .expect("client");

    let call = || {
        client
            .post(format!("{base}/mcp"))
            .json(&serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}))
            .send()
    };

    // max=3 → the first three pass the limiter; the fourth is throttled.
    for _ in 0..3 {
        let _ = call().await.expect("send");
    }
    let resp = call().await.expect("limited");
    assert_eq!(resp.status(), StatusCode::TOO_MANY_REQUESTS);
    assert!(resp.headers().get("retry-after").is_some());
    let body: serde_json::Value = resp.json().await.expect("jsonrpc body");
    assert_eq!(body["jsonrpc"], "2.0");
    assert_eq!(body["error"]["code"], -32029);
    assert!(
        body["error"]["data"]["retry_after_ms"].as_u64().is_some(),
        "backpressure envelope must carry retry_after_ms, got {body}"
    );
    handle.abort();
}

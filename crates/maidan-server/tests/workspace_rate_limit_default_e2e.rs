//! The per-workspace fairness limit is on by default, as the per-client limit
//! is: a server that configures nothing caps one workspace at 6000 requests a
//! minute across all its tokens, and another workspace keeps its own budget.
//! Its own test binary so the rate-limit environment it sets reaches no other
//! test.

use std::{net::SocketAddr, sync::Arc, time::Duration};

use futures::StreamExt;
use maidan_artifacts::LocalFsStore;
use maidan_server::{router, AppState};
use maidan_store::{configure_sqlite_pool, prelude::*, run_sqlite_migrations};
use reqwest::StatusCode;
use sqlx::sqlite::SqlitePoolOptions;

const DEFAULT_WORKSPACE_MAX: usize = 6000;

async fn spawn() -> (SocketAddr, tokio::task::JoinHandle<()>, tempfile::TempDir) {
    unsafe {
        // The per-client limit (1200 a minute) would stop this one client long
        // before the workspace default; turn it off to see the workspace's.
        std::env::set_var("MAIDAN_RATE_LIMIT_MAX", "0");
        std::env::remove_var("MAIDAN_WORKSPACE_RATE_LIMIT_MAX");
        std::env::remove_var("MAIDAN_WORKSPACE_RATE_LIMIT_WINDOW_SECS");
    }

    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    configure_sqlite_pool(&pool).await.expect("pragmas");
    run_sqlite_migrations(&pool).await.expect("migrate");
    let store = Arc::new(SqliteStore::for_tests(pool.clone()));
    let dir = tempfile::tempdir().expect("tempdir");
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let mut state = AppState::for_tests(store, artifacts, bus, search);
    // What the server bootstrap sets.
    state.rate_limit_default_on = true;
    let app = router(state);

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

async fn create_workspace(client: &reqwest::Client, base: &str, name: &str) -> String {
    let ws = client
        .post(format!("{base}/workspaces"))
        .json(&serde_json::json!({ "name": name }))
        .send()
        .await
        .expect("create ws")
        .json::<serde_json::Value>()
        .await
        .expect("json");
    ws["id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn an_unconfigured_server_caps_one_workspace_and_not_the_next() {
    let (addr, handle, _dir) = spawn().await;
    let base = format!("http://{addr}");
    let client = reqwest::Client::new();
    let a = create_workspace(&client, &base, "noisy").await;
    let b = create_workspace(&client, &base, "quiet").await;

    // Concurrently, so the whole budget is spent well inside the 60 s window
    // on a slow runner; the in-memory window opens on the first request.
    let url = format!("{base}/workspaces/{a}");
    let statuses: Vec<StatusCode> = futures::stream::iter(0..DEFAULT_WORKSPACE_MAX)
        .map(|_| {
            let (client, url) = (client.clone(), url.clone());
            async move { client.get(url).send().await.expect("get A").status() }
        })
        .buffer_unordered(32)
        .collect()
        .await;
    let refused = statuses.iter().filter(|s| **s != StatusCode::OK).count();
    assert_eq!(
        refused, 0,
        "all {DEFAULT_WORKSPACE_MAX} are within the default"
    );
    let limited = client
        .get(format!("{base}/workspaces/{a}"))
        .send()
        .await
        .expect("get A limited");
    assert_eq!(
        limited.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "the request past 6000 in a minute is refused with nothing configured"
    );

    let other = client
        .get(format!("{base}/workspaces/{b}"))
        .send()
        .await
        .expect("get B");
    assert_eq!(
        other.status(),
        StatusCode::OK,
        "another workspace has its own budget"
    );

    handle.abort();
}

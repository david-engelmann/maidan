//! Per-workspace fairness (Cluster 110): a workspace at its rate cap must not
//! degrade another workspace's requests. Its own test binary so the
//! `MAIDAN_WORKSPACE_RATE_LIMIT_*` env doesn't bleed into other rate-limit tests.

use std::{net::SocketAddr, sync::Arc, time::Duration};

use maidan_artifacts::LocalFsStore;
use maidan_server::{router, AppState};
use maidan_store::{configure_sqlite_pool, prelude::*, run_sqlite_migrations};
use reqwest::StatusCode;
use sqlx::sqlite::SqlitePoolOptions;

async fn spawn() -> (SocketAddr, tokio::task::JoinHandle<()>, tempfile::TempDir) {
    unsafe {
        // Per-workspace fairness on; global per-client limit left off so this
        // test isolates the workspace dimension.
        std::env::set_var("MAIDAN_WORKSPACE_RATE_LIMIT_MAX", "3");
        std::env::set_var("MAIDAN_WORKSPACE_RATE_LIMIT_WINDOW_SECS", "60");
        std::env::remove_var("MAIDAN_RATE_LIMIT_MAX");
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
async fn workspace_at_cap_does_not_starve_other_workspace() {
    let (addr, handle, _dir) = spawn().await;
    let base = format!("http://{addr}");
    let client = reqwest::Client::new();

    // Workspace creation is not workspace-scoped, so it isn't rate-limited.
    let a = create_workspace(&client, &base, "noisy").await;
    let b = create_workspace(&client, &base, "quiet").await;

    // Noisy workspace A: 3 reads allowed, the 4th hits the per-workspace cap.
    for _ in 0..3 {
        let resp = client
            .get(format!("{base}/workspaces/{a}"))
            .send()
            .await
            .expect("get A");
        assert_eq!(resp.status(), StatusCode::OK);
    }
    let limited = client
        .get(format!("{base}/workspaces/{a}"))
        .send()
        .await
        .expect("get A limited");
    assert_eq!(
        limited.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "workspace A should be capped"
    );

    // Quiet workspace B is unaffected — its bucket is independent.
    for _ in 0..3 {
        let resp = client
            .get(format!("{base}/workspaces/{b}"))
            .send()
            .await
            .expect("get B");
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "workspace B must not be starved by A hitting its cap"
        );
    }

    handle.abort();
}

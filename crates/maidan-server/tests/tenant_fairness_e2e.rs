//! Per-workspace fairness: a workspace at its rate cap must not degrade another
//! workspace's requests. Its own test binary so the
//! `MAIDAN_WORKSPACE_RATE_LIMIT_*` env doesn't bleed into other rate-limit
//! tests.

use std::{net::SocketAddr, sync::Arc};

use maidan_artifacts::LocalFsStore;
use maidan_server::{router, test_support, test_support::Tenant, AppState};
use maidan_store::{configure_sqlite_pool, prelude::*, run_sqlite_migrations};
use reqwest::StatusCode;

async fn spawn() -> (
    SocketAddr,
    tokio::task::JoinHandle<()>,
    tempfile::TempDir,
    Tenant,
    Tenant,
) {
    unsafe {
        // Per-workspace fairness on; global per-client limit left off so this
        // test isolates the workspace dimension.
        std::env::set_var("MAIDAN_WORKSPACE_RATE_LIMIT_MAX", "3");
        std::env::set_var("MAIDAN_WORKSPACE_RATE_LIMIT_WINDOW_SECS", "60");
        std::env::remove_var("MAIDAN_RATE_LIMIT_MAX");
    }

    let pool = maidan_search::sqlite_pool_options()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    configure_sqlite_pool(&pool).await.expect("pragmas");
    run_sqlite_migrations(&pool).await.expect("migrate");
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
    // Seeded through the store, so neither workspace spends its budget on setup.
    let noisy = test_support::tenant(store.as_ref(), "noisy").await;
    let quiet = test_support::tenant(store.as_ref(), "quiet").await;
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
    (addr, handle, dir, noisy, quiet)
}

#[tokio::test]
async fn workspace_at_cap_does_not_starve_other_workspace() {
    let (addr, handle, _dir, noisy, quiet) = spawn().await;
    let base = format!("http://{addr}");
    let a = noisy.workspace_id.0;
    let b = quiet.workspace_id.0;
    let client_a = noisy.client();
    let client_b = quiet.client();

    // Noisy workspace A: 3 reads allowed, the 4th hits the per-workspace cap.
    for _ in 0..3 {
        let resp = client_a
            .get(format!("{base}/workspaces/{a}"))
            .send()
            .await
            .expect("get A");
        assert_eq!(resp.status(), StatusCode::OK);
    }
    let limited = client_a
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
        let resp = client_b
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

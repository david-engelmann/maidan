//! Request body-size cap: a body over `MAIDAN_MAX_BODY_BYTES` is rejected with
//! 413 before the handler buffers it.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_server::{router, test_support, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use reqwest::StatusCode;

async fn spawn() -> (
    SocketAddr,
    reqwest::Client,
    test_support::Tenant,
    tokio::task::JoinHandle<()>,
) {
    let pool = maidan_search::sqlite_pool_options()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
    let tenant = test_support::tenant(store.as_ref(), "body-limit").await;
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    // The caller is authenticated so the request reaches the body extractor
    // (an anonymous one would 401 first). `router` reads
    // MAIDAN_MAX_BODY_BYTES at build time.
    let state = AppState::new(
        store,
        artifacts,
        bus,
        search,
        Arc::new(maidan_search::HashV1Provider),
        false,
        false,
        FederationRuntime::new(true, None),
        Arc::new(AtomicI64::new(0)),
        None,
    );
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = tenant.client();
    (addr, client, tenant, server)
}

#[tokio::test]
async fn oversized_request_body_is_rejected() {
    // Set the cap before the router is built (it reads env once at build).
    // SAFETY: test setup. Maidan reads this variable only through std,
    // whose environment lock serializes the read with this write.
    #[allow(unsafe_code)] // edition 2024 makes env writes unsafe; test setup only
    unsafe {
        std::env::set_var("MAIDAN_MAX_BODY_BYTES", "1024")
    };
    let (addr, client, tenant, server) = spawn().await;
    let base = format!("http://{addr}");
    let channels = format!("{base}/workspaces/{}/channels", tenant.workspace_id.0);

    // A ~4 KiB body to a JSON endpoint exceeds the 1 KiB cap → 413.
    let big = "x".repeat(4096);
    let over = client
        .post(&channels)
        .header("content-type", "application/json")
        .body(serde_json::json!({ "name": big }).to_string())
        .send()
        .await
        .unwrap();
    assert_eq!(over.status(), StatusCode::PAYLOAD_TOO_LARGE);

    // A small well-formed body is accepted.
    let ok = client
        .post(&channels)
        .json(&serde_json::json!({ "name": "small" }))
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status(), StatusCode::CREATED);

    // SAFETY: test setup. Maidan reads this variable only through std,
    // whose environment lock serializes the read with this write.
    #[allow(unsafe_code)] // edition 2024 makes env writes unsafe; test setup only
    unsafe {
        std::env::remove_var("MAIDAN_MAX_BODY_BYTES")
    };
    server.abort();
}

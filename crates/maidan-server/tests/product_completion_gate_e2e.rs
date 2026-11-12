//! Product completion gate: critical routes respond.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_server::{router, subscribe_resume, test_support, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use reqwest::StatusCode;

async fn spawn() -> (
    SocketAddr,
    reqwest::Client,
    tokio::task::JoinHandle<()>,
    test_support::Tenant,
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
    let tenant = test_support::tenant(store.as_ref(), "gate-ws").await;
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    let mut state = AppState::new(
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
    state.subscribe_resume_secret = Some(Arc::from(subscribe_resume::TEST_SUBSCRIBE_RESUME_SECRET));
    maidan_server::metrics::init();
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, tenant.client(), server, tenant)
}

#[tokio::test]
async fn completion_gate_health_ui_mcp_and_a2a_endpoints_exist() {
    let (addr, client, server, _tenant) = spawn().await;
    let base = format!("http://{addr}");

    assert_eq!(
        client
            .get(format!("{base}/health"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        client
            .get(format!("{base}/ui/"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        client
            .post(format!("{base}/mcp/streamable"))
            .body("{")
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let a2a_status = client
        .post(format!("{base}/a2a/v1/rpc"))
        .body("{}")
        .header("content-type", "application/json")
        .send()
        .await
        .unwrap()
        .status();
    assert_ne!(a2a_status, StatusCode::NOT_FOUND);

    server.abort();
}

#[tokio::test]
async fn completion_gate_v2_workspace_and_operator_surfaces_respond() {
    let (addr, client, server, tenant) = spawn().await;
    let base = format!("http://{addr}");
    let wid = tenant.workspace_id.0;

    assert_eq!(
        client
            .get(format!("{base}/openapi.json"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        client
            .get(format!("{base}/metrics"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        client
            .get(format!("{base}/workspaces/{wid}/apps"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        client
            .get(format!("{base}/workspaces/{wid}/webhooks"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        client
            .get(format!("{base}/workspaces/{wid}/app-installations"))
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );

    server.abort();
}

//! Product completion gate (Clusters 26 + 58): critical routes respond.

use std::{
    net::SocketAddr,
    sync::{atomic::AtomicI64, Arc},
};

use maidan_artifacts::LocalFsStore;
use maidan_server::{router, subscribe_resume, AppState, FederationRuntime};
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::NewWorkspace;
use reqwest::StatusCode;
use sqlx::sqlite::SqlitePoolOptions;

async fn spawn() -> (
    SocketAddr,
    reqwest::Client,
    tokio::task::JoinHandle<()>,
    Arc<dyn Store>,
) {
    let pool = SqlitePoolOptions::new()
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
    maidan_server::metrics::init();
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, reqwest::Client::new(), server, store)
}

#[tokio::test]
async fn completion_gate_health_ui_mcp_and_a2a_endpoints_exist() {
    let (addr, client, server, _store) = spawn().await;
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
    let (addr, client, server, store) = spawn().await;
    let base = format!("http://{addr}");

    let ws = store
        .create_workspace(NewWorkspace {
            name: "gate-ws".into(),
        })
        .await
        .unwrap();
    let wid = ws.id.0;

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

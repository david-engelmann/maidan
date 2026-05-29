//! `GET /openapi.json` returns a valid OpenAPI 3 document.

use std::{net::SocketAddr, sync::Arc, time::Duration};

use maidan_artifacts::LocalFsStore;
use maidan_server::{router, AppState};
use maidan_store::{run_sqlite_migrations, SqliteStore};
use sqlx::sqlite::SqlitePoolOptions;

async fn spawn() -> (SocketAddr, tokio::task::JoinHandle<()>, tempfile::TempDir) {
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("foreign_keys");
    run_sqlite_migrations(&pool).await.expect("migrate");

    let store = Arc::new(SqliteStore::new(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().expect("tempdir");
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    let app = router(AppState::for_tests(store, artifacts, bus, search));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, server, dir)
}

#[tokio::test]
async fn openapi_json_serves_document() {
    let (addr, server, _dir) = spawn().await;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("client");

    let resp = client
        .get(format!("http://{addr}/openapi.json"))
        .send()
        .await
        .expect("request");
    assert_eq!(resp.status(), 200);
    let doc: serde_json::Value = resp.json().await.expect("json");
    assert_eq!(doc["openapi"].as_str(), Some("3.0.3"));
    assert_eq!(doc["info"]["title"].as_str(), Some("Maidan API"));
    let paths = doc["paths"].as_object().expect("paths object");
    assert!(paths.contains_key("/health/live"));
    assert!(paths.contains_key("/workspaces"));
    assert!(paths.contains_key("/auth/oidc/login"));
    assert!(paths.contains_key("/auth/session"));
    assert!(paths.contains_key("/ui/api/workspaces/{wid}/events"));
    assert!(paths.contains_key("/ui/api/workspaces/{wid}/channels"));
    assert!(paths.contains_key("/ui/api/channels/{cid}/threads"));
    assert!(paths.contains_key("/ui/api/threads/{tid}/messages"));
    assert!(paths.contains_key("/ui/api/workspaces/{wid}/search"));
    assert!(!paths.contains_key("/mcp"));

    let schemes = doc["components"]["securitySchemes"]
        .as_object()
        .expect("securitySchemes");
    assert!(schemes.contains_key("sessionCookie"));

    server.abort();
}

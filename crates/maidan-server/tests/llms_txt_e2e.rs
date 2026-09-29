//! `GET /llms.txt` is the agent-facing index (llmstxt.org): public, Markdown,
//! and it names the paths and tools an agent needs to join the work loop.

use std::sync::Arc;

use maidan_artifacts::LocalFsStore;
use maidan_search::SqliteSearch;
use maidan_server::{router, AppState};
use maidan_store::{run_sqlite_migrations, SqliteStore};
use reqwest::StatusCode;
use sqlx::sqlite::SqlitePoolOptions;

#[tokio::test]
async fn llms_txt_is_public_markdown_that_names_the_work_loop() {
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store = Arc::new(SqliteStore::for_tests(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(SqliteSearch::new(pool));
    let dir = tempfile::tempdir().expect("tempdir");
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    let app = router(AppState::for_tests(store, artifacts, bus, search));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let res = reqwest::get(format!("http://{addr}/llms.txt"))
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK, "no token needed");
    let ctype = res.headers()["content-type"].to_str().unwrap().to_string();
    assert!(ctype.starts_with("text/markdown"), "{ctype}");
    let body = res.text().await.unwrap();
    assert!(body.starts_with("# Maidan\n"), "llms.txt opens with the H1");
    for needle in [
        "/mcp/streamable",
        "MCP-Protocol-Version: 2026-07-28",
        "is for provisioning",
        "/openapi.json",
        "claim_next_thread",
        "set_thread_result",
        "start_review",
        "lease_secs",
        "renew_claim",
        "release_claim",
        "get_waiting_inbox",
        "submit_review",
    ] {
        assert!(body.contains(needle), "llms.txt names {needle}");
    }

    server.abort();
}

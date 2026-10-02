//! Browser UI static assets served at `/ui/`.

use std::{net::SocketAddr, sync::Arc, time::Duration};

use maidan_artifacts::LocalFsStore;
use maidan_bus::InMemoryBus;
use maidan_server::{router, AppState};
use maidan_store::{prelude::*, run_sqlite_migrations};
use reqwest::StatusCode;
use sqlx::sqlite::SqlitePoolOptions;

async fn spawn() -> (SocketAddr, tokio::task::JoinHandle<()>) {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .expect("connect");
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .expect("foreign_keys");
    run_sqlite_migrations(&pool).await.expect("migrate");

    let store = Arc::new(SqliteStore::for_tests(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let artifacts = Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path()));
    let bus = Arc::new(InMemoryBus::new());
    let app = router(AppState::for_tests(store, artifacts, bus, search));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let addr = listener.local_addr().expect("addr");
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, server)
}

#[tokio::test]
async fn ui_index_returns_html_shell() {
    let (addr, server) = spawn().await;
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("client");

    let resp = client
        .get(format!("http://{addr}/ui/"))
        .send()
        .await
        .expect("GET /ui/");
    assert_eq!(resp.status(), StatusCode::OK);
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        content_type.contains("text/html"),
        "expected html content-type, got {content_type}"
    );

    let body = resp.text().await.expect("body");
    assert!(body.contains("<!DOCTYPE html>") || body.contains("<html"));
    assert!(body.contains("Maidan") || body.contains("maidan"));
    assert!(body.contains(r#"data-ui-version="8""#));
    assert!(body.contains(r#"id="channel-list""#));
    assert!(body.contains(r#"id="live-feed""#));
    assert!(body.contains(r#"/ui/static/main.js"#));
    assert!(body.contains(r#"/ui/static/board.css"#));

    let js = client
        .get(format!("http://{addr}/ui/static/main.js"))
        .send()
        .await
        .expect("GET /ui/static/main.js");
    assert_eq!(js.status(), StatusCode::OK);
    let js_type = js
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        js_type.contains("text/javascript"),
        "expected javascript content-type, got {js_type}"
    );
    let js_body = js.text().await.expect("main.js body");
    assert!(js_body.contains("start()"));

    let css = client
        .get(format!("http://{addr}/ui/static/board.css"))
        .send()
        .await
        .expect("GET /ui/static/board.css");
    assert_eq!(css.status(), StatusCode::OK);
    let css_type = css
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert!(
        css_type.contains("text/css"),
        "expected css content-type, got {css_type}"
    );
    let css_body = css.text().await.expect("board.css body");
    assert!(css_body.contains("#f7f5f0"));

    let missing = client
        .get(format!("http://{addr}/ui/static/no-such.js"))
        .send()
        .await
        .expect("GET missing asset");
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);

    server.abort();
}

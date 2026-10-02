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
    let csp = resp
        .headers()
        .get("content-security-policy")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    assert_eq!(csp, maidan_server::app::BOARD_UI_CSP);
    assert!(
        csp.split(';').any(|d| d.trim() == "script-src 'self'"),
        "script-src must be 'self' only, got {csp}"
    );
    assert!(
        !csp.split(';').any(|d| {
            let d = d.trim();
            (d.starts_with("script-src") || d.starts_with("style-src")) && d.contains("unsafe-")
        }),
        "board CSP must not allow unsafe script or style sources, got {csp}"
    );

    let bare = client
        .get(format!("http://{addr}/ui"))
        .send()
        .await
        .expect("GET /ui");
    assert_eq!(bare.status(), StatusCode::OK);
    assert_eq!(
        bare.headers()
            .get("content-security-policy")
            .and_then(|v| v.to_str().ok()),
        Some(maidan_server::app::BOARD_UI_CSP)
    );

    let body = resp.text().await.expect("body");
    assert!(body.contains("<!DOCTYPE html>") || body.contains("<html"));
    assert!(body.contains("Maidan") || body.contains("maidan"));
    assert!(body.contains(r#"data-ui-version="8""#));
    assert!(body.contains(r#"id="channel-list""#));
    assert!(body.contains(r#"id="live-feed""#));
    assert!(body.contains(r#"/ui/static/main.js"#));
    assert!(body.contains(r#"/ui/static/board.css"#));
    assert_eq!(
        body.matches("<script").count(),
        1,
        "the board document must keep a single script tag"
    );
    assert!(body.contains(r#"<script type="module" src="/ui/static/main.js"></script>"#));
    assert!(
        !body.to_ascii_lowercase().contains("javascript:"),
        "a javascript: URL would need a looser script-src"
    );
    assert!(
        !body.contains("style="),
        "a style attribute would be ignored under style-src 'self'"
    );

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
    assert_eq!(
        js.headers()
            .get("content-security-policy")
            .and_then(|v| v.to_str().ok()),
        Some(maidan_server::app::BOARD_UI_CSP)
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
    assert_eq!(
        css.headers()
            .get("content-security-policy")
            .and_then(|v| v.to_str().ok()),
        Some(maidan_server::app::BOARD_UI_CSP)
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

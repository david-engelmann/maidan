//! The React console served at `/console/`, beside the board at `/ui/`.

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

fn client() -> reqwest::Client {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .expect("client")
}

fn header(resp: &reqwest::Response, name: &str) -> String {
    resp.headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string()
}

#[tokio::test]
async fn console_index_is_the_built_shell_under_a_strict_csp() {
    let (addr, server) = spawn().await;
    for path in ["/ui", "/ui/"] {
        let resp = client()
            .get(format!("http://{addr}{path}"))
            .send()
            .await
            .expect("GET /ui");
        assert_eq!(resp.status(), StatusCode::OK, "{path}");
        assert!(header(&resp, "content-type").contains("text/html"));
        let csp = header(&resp, "content-security-policy");
        assert!(csp.contains("script-src 'self';"), "{csp}");
        assert!(csp.contains("style-src 'self';"), "{csp}");
        assert!(csp.contains("frame-ancestors 'none'"), "{csp}");
        assert!(!csp.contains("script-src 'self' 'unsafe-inline'"), "{csp}");
        let body = resp.text().await.expect("body");
        assert!(body.contains(r#"src="/ui/app.js""#), "{body}");
        assert!(body.contains(r#"href="/ui/style.css""#), "{body}");
        assert!(body.contains(r#"href="/ui/manifest.webmanifest""#), "{body}");
        assert!(!body.contains("<style"), "inline style element in {body}");
    }
    // The preview address points at /ui.
    let no_follow = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("client");
    for path in ["/console", "/console/"] {
        let old = no_follow
            .get(format!("http://{addr}{path}"))
            .send()
            .await
            .expect("GET /console");
        assert_eq!(old.status(), StatusCode::PERMANENT_REDIRECT, "{path}");
        assert_eq!(header(&old, "location"), "/ui/", "{path}");
    }
    server.abort();
}

#[tokio::test]
async fn console_assets_are_pinned_names_only() {
    let (addr, server) = spawn().await;
    for (name, kind) in [
        ("app.js", "text/javascript"),
        ("vendor.js", "text/javascript"),
        ("style.css", "text/css"),
        ("geist.woff2", "font/woff2"),
        ("icon.png", "image/png"),
        ("icon-192.png", "image/png"),
        ("icon-512.png", "image/png"),
        ("manifest.webmanifest", "application/manifest+json"),
        ("sw.js", "text/javascript"),
    ] {
        let resp = client()
            .get(format!("http://{addr}/ui/{name}"))
            .send()
            .await
            .expect("GET asset");
        assert_eq!(resp.status(), StatusCode::OK, "{name}");
        assert!(header(&resp, "content-type").starts_with(kind), "{name}");
        assert!(!header(&resp, "etag").is_empty(), "{name} has no etag");
    }
    let missing = client()
        .get(format!("http://{addr}/ui/index.js"))
        .send()
        .await
        .expect("GET missing");
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    server.abort();
}

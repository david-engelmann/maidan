//! ETag revalidation on the board page and its assets: a first answer
//! carries a strong ETag and a short cache lifetime; a revalidation with
//! that tag answers 304 with no body, and a stale tag gets the full body
//! again. Before this, every board load re-downloaded every module.

use std::sync::{atomic::AtomicI64, Arc};

use maidan_artifacts::LocalFsStore;
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use reqwest::StatusCode;
use sqlx::sqlite::SqlitePoolOptions;

async fn spawn() -> String {
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
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let state = AppState::new(
        store,
        Arc::new(LocalFsStore::new(dir.path())),
        Arc::new(maidan_bus::InMemoryBus::new()),
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
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

/// The round trip every surface must honour. Returns the ETag.
async fn assert_revalidates(client: &reqwest::Client, url: &str) -> String {
    let first = client.get(url).send().await.unwrap();
    assert_eq!(first.status(), StatusCode::OK, "{url} answers");
    let etag = first
        .headers()
        .get("etag")
        .expect("the answer carries an ETag")
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        etag.starts_with('"') && etag.ends_with('"'),
        "the ETag is a quoted strong tag: {etag}"
    );
    let cache_control = first
        .headers()
        .get("cache-control")
        .expect("the answer carries a cache lifetime")
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        cache_control.contains("max-age=300"),
        "the lifetime is the short one: {cache_control}"
    );
    let body = first.text().await.unwrap();
    assert!(!body.is_empty(), "{url} answers with its body");

    let revalidated = client
        .get(url)
        .header("if-none-match", &etag)
        .send()
        .await
        .unwrap();
    assert_eq!(
        revalidated.status(),
        StatusCode::NOT_MODIFIED,
        "a current tag revalidates to a bodyless 304"
    );
    assert_eq!(
        revalidated
            .headers()
            .get("etag")
            .map(|v| v.to_str().unwrap()),
        Some(etag.as_str()),
        "the 304 repeats the tag"
    );
    assert_eq!(
        revalidated.text().await.unwrap(),
        "",
        "the 304 carries no body"
    );

    let stale = client
        .get(url)
        .header("if-none-match", "\"a-tag-from-an-older-build\"")
        .send()
        .await
        .unwrap();
    assert_eq!(stale.status(), StatusCode::OK, "a stale tag gets the body");
    assert_eq!(
        stale.text().await.unwrap(),
        body,
        "the full body, unchanged"
    );
    etag
}

#[tokio::test]
async fn the_board_page_and_its_assets_revalidate_by_etag() {
    let base = spawn().await;
    let client = reqwest::Client::new();
    assert_revalidates(&client, &format!("{base}/ui/")).await;
    let script_tag = assert_revalidates(&client, &format!("{base}/ui/static/main.js")).await;
    let style_tag = assert_revalidates(&client, &format!("{base}/ui/static/board.css")).await;
    assert_ne!(script_tag, style_tag, "each body carries its own tag");
}

#[tokio::test]
async fn an_unknown_asset_still_answers_404() {
    let base = spawn().await;
    let response = reqwest::Client::new()
        .get(format!("{base}/ui/static/nope.js"))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

//! OAuth phase one over HTTP: with `MAIDAN_PUBLIC_ORIGIN` unset the instance
//! serves no OAuth metadata and challenges nothing. With it set, the MCP
//! endpoint's RFC 9728 document names the configured origin, a 401 from an MCP
//! route points at it, and a 401 elsewhere does not.

use std::sync::{atomic::AtomicI64, Arc};

use maidan_artifacts::LocalFsStore;
use maidan_server::{router, AppState, FederationRuntime};
use maidan_store::{prelude::*, run_sqlite_migrations};
use reqwest::{header::WWW_AUTHENTICATE, StatusCode};
use serde_json::Value;
use sqlx::sqlite::SqlitePoolOptions;

async fn spawn(public_origin: Option<&str>) -> String {
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
    let mut state = AppState::new(
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
    state.public_origin = public_origin.map(str::to_string);
    let app = router(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{addr}")
}

#[tokio::test]
async fn without_a_public_origin_no_oauth_metadata_is_served_or_pointed_at() {
    let base = spawn(None).await;
    let client = reqwest::Client::new();
    for path in [
        "/.well-known/oauth-protected-resource",
        "/.well-known/oauth-protected-resource/mcp/streamable",
        "/.well-known/oauth-authorization-server",
    ] {
        let s = client
            .get(format!("{base}{path}"))
            .send()
            .await
            .unwrap()
            .status();
        assert_eq!(s, StatusCode::NOT_FOUND, "{path}");
    }
    let refused = client
        .post(format!("{base}/mcp/streamable"))
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);
    assert!(refused.headers().get(WWW_AUTHENTICATE).is_none());
}

#[tokio::test]
async fn with_a_public_origin_the_mcp_resource_describes_itself_and_its_401_points_there() {
    let origin = "https://maidan.example.com";
    let base = spawn(Some(origin)).await;
    let client = reqwest::Client::new();

    // RFC 9728 section 3.1: the metadata lives at the well-known prefix
    // inserted before the resource's path, and a client rejects a document
    // whose `resource` is not the identifier it derived that URL from.
    let resource = "https://maidan.example.com/mcp/streamable";
    let doc: Value = client
        .get(format!(
            "{base}/.well-known/oauth-protected-resource/mcp/streamable"
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(doc["resource"], resource);
    assert!(doc.get("authorization_servers").is_none(), "{doc}");
    // The root form would describe the resource `<origin>`, which this
    // document is not, so it is not served.
    let root = client
        .get(format!("{base}/.well-known/oauth-protected-resource"))
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(root, StatusCode::NOT_FOUND);
    // No authorization-server document until the flows exist.
    let s = client
        .get(format!("{base}/.well-known/oauth-authorization-server"))
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(s, StatusCode::NOT_FOUND);

    let expected = "Bearer resource_metadata=\"https://maidan.example.com/.well-known/oauth-protected-resource/mcp/streamable\"";
    let refused = client
        .post(format!("{base}/mcp/streamable"))
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status(), StatusCode::UNAUTHORIZED);
    assert_eq!(
        refused
            .headers()
            .get(WWW_AUTHENTICATE)
            .map(|v| v.to_str().unwrap()),
        Some(expected)
    );
    // `/mcp` is another URL than the resource the metadata names, so a client
    // following a challenge from it would have to reject the document.
    let legacy = client
        .post(format!("{base}/mcp"))
        .body("{}")
        .send()
        .await
        .unwrap();
    assert_eq!(legacy.status(), StatusCode::UNAUTHORIZED);
    assert!(legacy.headers().get(WWW_AUTHENTICATE).is_none());
    // A 401 that is not about an MCP bearer token carries no challenge.
    let elsewhere = client
        .get(format!(
            "{base}/workspaces/{}/channels",
            uuid::Uuid::now_v7()
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(elsewhere.status(), StatusCode::UNAUTHORIZED);
    assert!(elsewhere.headers().get(WWW_AUTHENTICATE).is_none());
}

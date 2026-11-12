//! UI v3 collaboration: ui/api reads for threads, messages, faceted search.

use std::sync::Arc;

use maidan_artifacts::LocalFsStore;
use maidan_bus::InMemoryBus;
use maidan_server::{router, test_support, test_support::Tenant, AppState};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::MemberKind;
use reqwest::StatusCode;
use serde_json::json;

async fn spawn() -> (
    std::net::SocketAddr,
    reqwest::Client,
    Arc<dyn Store>,
    Tenant,
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
    let search: Arc<dyn maidan_search::Search> = Arc::new(maidan_search::SqliteSearch::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(InMemoryBus::with_capacity(64));
    let tenant = test_support::tenant(store.as_ref(), "ui-collab").await;
    let app = router(AppState::for_tests(store.clone(), artifacts, bus, search));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (addr, tenant.client(), store, tenant, server)
}

#[tokio::test]
async fn ui_v3_collab_shell_and_session_api_reads() {
    let (addr, client, store, tenant, server) = spawn().await;
    let base = format!("http://{addr}");

    let html = client
        .get(format!("{base}/ui/"))
        .send()
        .await
        .unwrap()
        .text()
        .await
        .unwrap();
    // The console draws this panel in the browser; the served shell is its
    // React root, and the browser specs pin the panel itself.
    assert!(html.contains(r#"id="root""#), "{html}");
    assert!(html.contains(r#"src="/ui/app.js""#), "{html}");

    let workspace_id = tenant.workspace_id.0;
    let alice = tenant
        .member(store.as_ref(), "alice", MemberKind::Human)
        .await;

    let channel: serde_json::Value = client
        .post(format!("{base}/workspaces/{workspace_id}/channels"))
        .json(&json!({"name": "general"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let channel_id = channel["id"].as_str().unwrap();

    let thread: serde_json::Value = client
        .post(format!("{base}/channels/{channel_id}/threads"))
        .json(&json!({"title": "collab-thread"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let thread_id = thread["id"].as_str().unwrap();

    let msg: serde_json::Value = alice
        .client()
        .post(format!("{base}/threads/{thread_id}/messages"))
        .json(&json!({"body": "faceted search needle collab-v44"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(msg["body"], "faceted search needle collab-v44");

    let threads: Vec<serde_json::Value> = client
        .get(format!("{base}/ui/api/channels/{channel_id}/threads"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(threads.len(), 1);
    assert_eq!(threads[0]["title"], "collab-thread");

    let messages: Vec<serde_json::Value> = client
        .get(format!(
            "{base}/ui/api/threads/{thread_id}/messages?limit=10"
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(messages.len(), 1);

    let hits: Vec<serde_json::Value> = client
        .get(format!(
            "{base}/ui/api/workspaces/{workspace_id}/search?q=needle&mode=lexical&channel={channel_id}&limit=10"
        ))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        !hits.is_empty(),
        "expected lexical hit for channel-filtered search"
    );

    let artifact = client
        .post(format!(
            "{base}/artifacts?kind=attachment&mime_type=text/plain"
        ))
        .body(b"ui-collab artifact bytes".to_vec())
        .send()
        .await
        .unwrap();
    assert_eq!(artifact.status(), StatusCode::CREATED);

    server.abort();
}

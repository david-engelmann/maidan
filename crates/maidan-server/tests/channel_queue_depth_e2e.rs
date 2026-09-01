//! Cluster 224: `GET /channels/:cid/queue-depth` reports ready / assigned /
//! blocked counts of a channel's open task threads over HTTP.

use std::{sync::Arc, time::Duration};

use maidan_artifacts::LocalFsStore;
use maidan_bus::InMemoryBus;
use maidan_server::{router, AppState};
use maidan_store::{prelude::*, run_sqlite_migrations};
use serde_json::json;
use sqlx::sqlite::SqlitePoolOptions;

#[tokio::test]
async fn queue_depth_partitions_ready_and_blocked_threads() {
    let pool = SqlitePoolOptions::new()
        .max_connections(4)
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
    let bus = Arc::new(InMemoryBus::with_capacity(64));

    let app = router(AppState::for_tests(store, artifacts, bus, search));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let _server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let base = format!("http://{addr}");
    let post = |path: String, body: serde_json::Value| {
        let client = client.clone();
        async move {
            client
                .post(path)
                .json(&body)
                .send()
                .await
                .unwrap()
                .json::<serde_json::Value>()
                .await
                .unwrap()
        }
    };

    let ws = post(format!("{base}/workspaces"), json!({"name": "qd-ws"})).await;
    let wid = ws["id"].as_str().unwrap();
    let ch = post(
        format!("{base}/workspaces/{wid}/channels"),
        json!({"name": "queue"}),
    )
    .await;
    let cid = ch["id"].as_str().unwrap();

    // ready1 + dep have no dependencies → ready. blocked1 depends on dep.
    let _ready1 = post(
        format!("{base}/channels/{cid}/threads"),
        json!({"title": "ready1"}),
    )
    .await;
    let dep = post(
        format!("{base}/channels/{cid}/threads"),
        json!({"title": "dep"}),
    )
    .await;
    let blocked1 = post(
        format!("{base}/channels/{cid}/threads"),
        json!({"title": "blocked1"}),
    )
    .await;
    let add = client
        .post(format!(
            "{base}/threads/{}/dependencies",
            blocked1["id"].as_str().unwrap()
        ))
        .json(&json!({"depends_on_thread_id": dep["id"].as_str().unwrap()}))
        .send()
        .await
        .unwrap();
    assert_eq!(add.status(), 204);

    let depth: serde_json::Value = client
        .get(format!("{base}/channels/{cid}/queue-depth"))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(depth["open"], json!(3));
    assert_eq!(depth["ready"], json!(2), "ready1 + dep");
    assert_eq!(depth["blocked"], json!(1), "blocked1 waits on dep");
    assert_eq!(depth["assigned"], json!(0));
}

//! Artifact upload/download HTTP round-trip.

use std::{net::SocketAddr, sync::Arc, time::Duration};

use maidan_artifacts::LocalFsStore;
use maidan_server::{router, AppState};
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use reqwest::StatusCode;
use sqlx::sqlite::SqlitePoolOptions;

#[tokio::test]
async fn upload_and_download_artifact_round_trip() {
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

    let search: Arc<dyn maidan_search::Search> =
        Arc::new(maidan_search::SqliteSearch::new(pool.clone()));
    let store: Arc<dyn Store> = Arc::new(SqliteStore::new(pool));
    let dir = tempfile::tempdir().unwrap();
    let artifacts = Arc::new(LocalFsStore::new(dir.path()));
    let bus = Arc::new(maidan_bus::InMemoryBus::new());
    let app = router(AppState::new(store, artifacts, bus, search));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
        .unwrap();
    let base = format!("http://{addr}");

    let payload = b"screenshot bytes for cluster e";
    let created = client
        .post(format!(
            "{base}/artifacts?kind=screenshot&mime_type=image/png"
        ))
        .body(payload.to_vec())
        .send()
        .await
        .unwrap();
    assert_eq!(created.status(), StatusCode::CREATED);
    let artifact: serde_json::Value = created.json().await.unwrap();
    let sha = artifact["sha256"].as_str().unwrap();

    let meta = client
        .get(format!("{base}/artifacts/{sha}/meta"))
        .send()
        .await
        .unwrap();
    assert_eq!(meta.status(), StatusCode::OK);
    assert_eq!(
        meta.json::<serde_json::Value>().await.unwrap()["kind"],
        "screenshot"
    );

    let body = client
        .get(format!("{base}/artifacts/{sha}"))
        .send()
        .await
        .unwrap();
    assert_eq!(body.status(), StatusCode::OK);
    assert_eq!(body.bytes().await.unwrap().as_ref(), payload.as_slice());

    server.abort();
}

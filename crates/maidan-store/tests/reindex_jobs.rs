//! Reindex job store (Cluster 104.0.3): upsert is keyed by job_id so a Running
//! record and its later terminal update collapse to one row; both backends
//! behave identically.

use std::time::Duration;

use chrono::Utc;
use maidan_store::{
    run_postgres_migrations, run_sqlite_migrations, PostgresStore, SqliteStore, Store,
};
use maidan_types::*;
use sqlx::postgres::PgPoolOptions;
use sqlx::sqlite::SqlitePoolOptions;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

/// Shared assertions exercised against both backends.
async fn assert_upsert_and_get(store: &dyn Store) {
    let job_id = uuid::Uuid::new_v4();
    let started = Utc::now();

    // Unknown job is absent.
    assert!(store.get_reindex_job(job_id).await.unwrap().is_none());

    // Insert a Running record.
    store
        .upsert_reindex_job(ReindexJob {
            job_id,
            status: ReindexJobStatus::Running,
            workspace_id: None,
            embedding_model: "hash-v1".into(),
            processed: None,
            failed: None,
            error: None,
            started_at: started,
            finished_at: None,
        })
        .await
        .unwrap();

    let got = store.get_reindex_job(job_id).await.unwrap().unwrap();
    assert!(matches!(got.status, ReindexJobStatus::Running));
    assert_eq!(got.embedding_model, "hash-v1");
    assert!(got.processed.is_none());
    assert!(got.finished_at.is_none());

    // Terminal update upserts the same row (no duplicate, mutable fields change).
    let finished = Utc::now();
    store
        .upsert_reindex_job(ReindexJob {
            job_id,
            status: ReindexJobStatus::Completed,
            workspace_id: None,
            embedding_model: "hash-v1".into(),
            processed: Some(42),
            failed: Some(1),
            error: None,
            started_at: started,
            finished_at: Some(finished),
        })
        .await
        .unwrap();

    let got = store.get_reindex_job(job_id).await.unwrap().unwrap();
    assert!(matches!(got.status, ReindexJobStatus::Completed));
    assert_eq!(got.processed, Some(42));
    assert_eq!(got.failed, Some(1));
    assert!(got.finished_at.is_some());
}

#[tokio::test]
async fn reindex_job_upsert_and_get_sqlite() {
    let pool = SqlitePoolOptions::new()
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(&pool)
        .await
        .unwrap();
    run_sqlite_migrations(&pool).await.unwrap();
    let store = SqliteStore::new(pool);
    assert_upsert_and_get(&store).await;
}

#[tokio::test]
async fn reindex_job_upsert_and_get_postgres() {
    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            eprintln!("skipping reindex_jobs postgres: docker unavailable ({err})");
            return;
        }
    };
    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .unwrap();
    run_postgres_migrations(&pool).await.unwrap();
    let store = PostgresStore::new(pool);
    assert_upsert_and_get(&store).await;
}

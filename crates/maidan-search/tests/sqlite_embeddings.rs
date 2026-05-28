//! sqlite-vec-backed embedding round-trip + semantic ranking on SQLite.

mod common;

use std::sync::Arc;

use maidan_search::{postgres::EMBEDDING_DIM, Search, SearchFilters, SqliteSearch};
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use maidan_types::MessageId;
use sqlx::sqlite::SqlitePoolOptions;

fn one_hot(index: usize) -> Vec<f32> {
    let mut v = vec![0.0; EMBEDDING_DIM];
    v[index] = 1.0;
    v
}

#[tokio::test]
async fn semantic_search_orders_by_cosine_distance_on_sqlite() {
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
    let search = SqliteSearch::new(pool);
    let fx = common::seed(&*store).await;

    let alive_ids: Vec<MessageId> = fx
        .message_ids
        .iter()
        .copied()
        .filter(|id| *id != fx.tombstoned)
        .collect();
    assert!(alive_ids.len() >= 3);

    for (i, id) in alive_ids.iter().take(3).enumerate() {
        search
            .upsert_embedding(*id, "test-model", &one_hot(i))
            .await
            .unwrap();
    }

    let hits = search
        .semantic_search(
            fx.workspace_id,
            &one_hot(1),
            3,
            &SearchFilters::default(),
            "test-model",
        )
        .await
        .unwrap();
    assert_eq!(hits.len(), 3);
    assert_eq!(hits[0].message_id, alive_ids[1]);
    assert!((hits[0].rank - 1.0).abs() < 1e-6);
}

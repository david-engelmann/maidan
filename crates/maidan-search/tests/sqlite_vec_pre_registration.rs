//! Semantic search on a pool holding a connection opened before sqlite-vec
//! was registered for the process.
//!
//! This is its own test binary so nothing registers the process-wide
//! auto-extension before the old connection is opened.

mod common;

use std::sync::Arc;

use maidan_search::{
    ensure_auto_extension, postgres::EMBEDDING_DIM, Search, SearchFilters, SqliteSearch,
};
use maidan_store::{prelude::*, run_sqlite_migrations};
use maidan_types::MessageId;
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};

fn one_hot(index: usize) -> Vec<f32> {
    let mut v = vec![0.0; EMBEDDING_DIM];
    v[index] = 1.0;
    v
}

#[tokio::test]
async fn semantic_search_succeeds_on_a_pool_whose_connection_predates_registration() {
    let dir = std::env::temp_dir().join(format!("maidan_vec_prereg_{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("search.db");
    let _ = std::fs::remove_file(&path);
    let options = SqliteConnectOptions::new()
        .filename(&path)
        .create_if_missing(true)
        .foreign_keys(true);
    // A bare pool, as a caller that skips `sqlite_pool_options` builds it.
    let pool = SqlitePoolOptions::new()
        .max_connections(2)
        .connect_with(options)
        .await
        .unwrap();

    // `old` is opened before registration and has no sqlite-vec.
    let old = pool.acquire().await.unwrap();
    ensure_auto_extension();
    let new = pool.acquire().await.unwrap();
    // Return `new` first: the pool hands idle connections out in order, so
    // a check on the pool lands on `new` and the next query on `old`.
    drop(new);
    drop(old);

    run_sqlite_migrations(&pool).await.unwrap();
    let store: Arc<dyn Store> = Arc::new(SqliteStore::for_tests(pool.clone()));
    let search = SqliteSearch::new(pool.clone());
    let fx = common::seed(&*store).await;
    let alive: Vec<MessageId> = fx
        .message_ids
        .iter()
        .copied()
        .filter(|id| *id != fx.tombstoned)
        .collect();
    for (i, id) in alive.iter().take(3).enumerate() {
        search
            .upsert_embedding(*id, "test-model", &one_hot(i))
            .await
            .unwrap();
    }

    // Twice, so both pooled connections serve a search.
    for round in 0..2 {
        let hits = search
            .semantic_search(
                fx.workspace_id,
                &one_hot(1),
                3,
                &SearchFilters::default(),
                "test-model",
            )
            .await
            .unwrap_or_else(|e| panic!("round {round}: semantic search failed: {e}"));
        assert_eq!(hits.len(), 3, "round {round}");
        assert_eq!(hits[0].message_id, alive[1], "round {round}");
    }
    pool.close().await;
    let _ = std::fs::remove_dir_all(&dir);
}

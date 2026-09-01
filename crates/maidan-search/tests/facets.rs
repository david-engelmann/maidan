//! Faceted lexical search (author, channel, member kind).

mod common;

use std::sync::Arc;

use maidan_search::SqliteSearch;
use maidan_store::{prelude::*, run_sqlite_migrations};
use sqlx::sqlite::SqlitePoolOptions;

#[tokio::test]
async fn sqlite_faceted_search_filters_hits() {
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
    common::assert_faceted_search(&search, &fx).await;
    common::assert_deny_channels_filter(&search, &fx).await;
}

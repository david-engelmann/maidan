//! SqliteSearch integration test.

mod common;

use std::sync::Arc;

use maidan_search::SqliteSearch;
use maidan_store::{run_sqlite_migrations, SqliteStore, Store};
use sqlx::sqlite::SqlitePoolOptions;

#[tokio::test]
async fn full_text_search_against_sqlite() {
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
    common::run_search_suite(&search, &fx).await;
}

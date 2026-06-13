//! Connection PRAGMAs for file-backed and in-memory SQLite pools.

use sqlx::SqlitePool;

use crate::error::StoreError;

/// `foreign_keys`, WAL journal, and a 5000 ms `busy_timeout` for concurrent
/// readers/writers.
pub async fn configure_pool(pool: &SqlitePool) -> Result<(), StoreError> {
    configure_pool_with(pool, 5000).await
}

/// As [`configure_pool`], with a configurable `busy_timeout` in ms (Cluster 107).
pub async fn configure_pool_with(
    pool: &SqlitePool,
    busy_timeout_ms: u64,
) -> Result<(), StoreError> {
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(pool)
        .await?;
    sqlx::query("PRAGMA journal_mode = WAL")
        .execute(pool)
        .await?;
    sqlx::query(&format!("PRAGMA busy_timeout = {busy_timeout_ms}"))
        .execute(pool)
        .await?;
    Ok(())
}

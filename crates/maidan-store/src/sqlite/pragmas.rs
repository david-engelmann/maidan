//! Connection PRAGMAs for file-backed and in-memory SQLite pools.

use sqlx::SqlitePool;

use crate::error::StoreError;

/// `foreign_keys`, WAL journal, and `busy_timeout` for concurrent readers/writers.
pub async fn configure_pool(pool: &SqlitePool) -> Result<(), StoreError> {
    sqlx::query("PRAGMA foreign_keys = ON")
        .execute(pool)
        .await?;
    sqlx::query("PRAGMA journal_mode = WAL")
        .execute(pool)
        .await?;
    sqlx::query("PRAGMA busy_timeout = 5000")
        .execute(pool)
        .await?;
    Ok(())
}

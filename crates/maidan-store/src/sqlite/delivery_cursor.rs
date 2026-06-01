//! SQLite per-consumer delivery cursors (`log_id` watermark).

use maidan_types::WorkspaceId;
use sqlx::SqlitePool;

use crate::error::StoreError;

pub async fn get_cursor(
    pool: &SqlitePool,
    consumer_id: &str,
    workspace_id: WorkspaceId,
) -> Result<i64, StoreError> {
    let row: Option<(i64,)> = sqlx::query_as(
        "SELECT last_delivered_log_id
         FROM maidan_delivery_cursor
         WHERE consumer_id = ? AND workspace_id = ?",
    )
    .bind(consumer_id)
    .bind(workspace_id.0)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| r.0).unwrap_or(0))
}

/// Advances the cursor to `log_id` only when `log_id` is greater than the stored value.
pub async fn advance_cursor(
    pool: &SqlitePool,
    consumer_id: &str,
    workspace_id: WorkspaceId,
    log_id: i64,
) -> Result<i64, StoreError> {
    let existing = get_cursor(pool, consumer_id, workspace_id).await?;
    let new_id = log_id.max(existing);
    sqlx::query(
        "INSERT INTO maidan_delivery_cursor (consumer_id, workspace_id, last_delivered_log_id, updated_at)
         VALUES (?, ?, ?, CURRENT_TIMESTAMP)
         ON CONFLICT (consumer_id, workspace_id) DO UPDATE SET
           last_delivered_log_id = CASE
             WHEN excluded.last_delivered_log_id > maidan_delivery_cursor.last_delivered_log_id
             THEN excluded.last_delivered_log_id
             ELSE maidan_delivery_cursor.last_delivered_log_id
           END,
           updated_at = CURRENT_TIMESTAMP",
    )
    .bind(consumer_id)
    .bind(workspace_id.0)
    .bind(new_id)
    .execute(pool)
    .await?;
    get_cursor(pool, consumer_id, workspace_id).await
}

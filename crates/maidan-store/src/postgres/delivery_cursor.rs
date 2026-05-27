//! Postgres per-consumer delivery cursors (`log_id` watermark).

use maidan_types::WorkspaceId;
use sqlx::PgPool;

use crate::error::StoreError;

pub async fn get_cursor(
    pool: &PgPool,
    consumer_id: &str,
    workspace_id: WorkspaceId,
) -> Result<i64, StoreError> {
    let row: Option<(i64,)> = sqlx::query_as(
        "SELECT last_delivered_log_id
         FROM maidan_delivery_cursor
         WHERE consumer_id = $1 AND workspace_id = $2",
    )
    .bind(consumer_id)
    .bind(workspace_id.0)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| r.0).unwrap_or(0))
}

/// Advances the cursor to `log_id` only when `log_id` is greater than the stored value.
pub async fn advance_cursor(
    pool: &PgPool,
    consumer_id: &str,
    workspace_id: WorkspaceId,
    log_id: i64,
) -> Result<i64, StoreError> {
    let row: (i64,) = sqlx::query_as(
        "INSERT INTO maidan_delivery_cursor (consumer_id, workspace_id, last_delivered_log_id, updated_at)
         VALUES ($1, $2, $3, NOW())
         ON CONFLICT (consumer_id, workspace_id)
         DO UPDATE SET
           last_delivered_log_id = GREATEST(maidan_delivery_cursor.last_delivered_log_id, EXCLUDED.last_delivered_log_id),
           updated_at = NOW()
         RETURNING last_delivered_log_id",
    )
    .bind(consumer_id)
    .bind(workspace_id.0)
    .bind(log_id)
    .fetch_one(pool)
    .await?;
    Ok(row.0)
}

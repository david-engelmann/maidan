//! SQLite data-retention pruning (Cluster 186). Batched deletes (subquery
//! `LIMIT`) so a first sweep over a long-unpruned table doesn't lock it.

use chrono::{DateTime, Utc};
use sqlx::SqlitePool;

use crate::error::StoreError;

pub async fn min_delivery_cursor(pool: &SqlitePool) -> Result<Option<i64>, StoreError> {
    let row: Option<(Option<i64>,)> =
        sqlx::query_as("SELECT MIN(last_delivered_log_id) FROM maidan_delivery_cursor")
            .fetch_optional(pool)
            .await?;
    Ok(row.and_then(|r| r.0))
}

pub async fn prune_events(
    pool: &SqlitePool,
    cutoff: DateTime<Utc>,
    max_id: i64,
    limit: i64,
) -> Result<u64, StoreError> {
    let res = sqlx::query(
        "DELETE FROM maidan_events
         WHERE id IN (
             SELECT id FROM maidan_events
             WHERE id <= ? AND occurred_at < ?
             ORDER BY id ASC
             LIMIT ?
         )",
    )
    .bind(max_id)
    .bind(cutoff)
    .bind(limit)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

pub async fn prune_audit(
    pool: &SqlitePool,
    cutoff: DateTime<Utc>,
    limit: i64,
) -> Result<u64, StoreError> {
    let res = sqlx::query(
        "DELETE FROM maidan_audit
         WHERE id IN (
             SELECT id FROM maidan_audit
             WHERE occurred_at < ?
             ORDER BY id ASC
             LIMIT ?
         )",
    )
    .bind(cutoff)
    .bind(limit)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

pub async fn prune_deliveries(
    pool: &SqlitePool,
    cutoff: DateTime<Utc>,
    limit: i64,
) -> Result<u64, StoreError> {
    let mut total = 0u64;
    for table in ["maidan_webhook_deliveries", "maidan_automation_deliveries"] {
        // Only terminal rows (delivered or quarantined); in-flight rows stay.
        let sql = format!(
            "DELETE FROM {table}
             WHERE id IN (
                 SELECT id FROM {table}
                 WHERE created_at < ?
                   AND (delivered_at IS NOT NULL OR quarantined_at IS NOT NULL)
                 ORDER BY id ASC
                 LIMIT ?
             )"
        );
        let res = sqlx::query(&sql)
            .bind(cutoff)
            .bind(limit)
            .execute(pool)
            .await?;
        total += res.rows_affected();
    }
    Ok(total)
}

//! Postgres data-retention pruning (Cluster 186). Batched deletes (subquery
//! `LIMIT`) so a first sweep over a long-unpruned table doesn't lock it.

use chrono::{DateTime, Utc};
use sqlx::PgPool;

use crate::error::StoreError;

pub async fn min_delivery_cursor(pool: &PgPool) -> Result<Option<i64>, StoreError> {
    let row: (Option<i64>,) =
        sqlx::query_as("SELECT MIN(last_delivered_log_id) FROM maidan_delivery_cursor")
            .fetch_one(pool)
            .await?;
    Ok(row.0)
}

pub async fn prune_events(
    pool: &PgPool,
    cutoff: DateTime<Utc>,
    max_id: i64,
    limit: i64,
) -> Result<u64, StoreError> {
    let res = sqlx::query(
        "DELETE FROM maidan_events
         WHERE id IN (
             SELECT id FROM maidan_events
             WHERE id <= $1 AND occurred_at < $2
             ORDER BY id ASC
             LIMIT $3
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
    pool: &PgPool,
    cutoff: DateTime<Utc>,
    limit: i64,
) -> Result<u64, StoreError> {
    let res = sqlx::query(
        "DELETE FROM maidan_audit
         WHERE id IN (
             SELECT id FROM maidan_audit
             WHERE occurred_at < $1
             ORDER BY id ASC
             LIMIT $2
         )",
    )
    .bind(cutoff)
    .bind(limit)
    .execute(pool)
    .await?;
    Ok(res.rows_affected())
}

pub async fn prune_deliveries(
    pool: &PgPool,
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
                 WHERE created_at < $1
                   AND (delivered_at IS NOT NULL OR quarantined_at IS NOT NULL)
                 ORDER BY id ASC
                 LIMIT $2
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

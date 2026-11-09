//! Operator alerts that stand for a condition: a row per key while it
//! stands, so a process that restarts mid-outage does not post it again.

use chrono::Utc;
use sqlx::SqlitePool;

use crate::error::StoreError;

/// Stand `key` at `value`; true when it did not already stand at `value`
/// (absent, or at another value), which is when the caller posts. The
/// conditional upsert makes the check and the write one statement, so two
/// processes raising together post once.
pub async fn raise(pool: &SqlitePool, key: &str, value: &str) -> Result<bool, StoreError> {
    let row = sqlx::query(
        "INSERT INTO maidan_ops_alerts (alert_key, value, raised_at)
         VALUES (?, ?, ?)
         ON CONFLICT (alert_key) DO UPDATE
            SET value = excluded.value, raised_at = excluded.raised_at
            WHERE maidan_ops_alerts.value <> excluded.value
         RETURNING alert_key",
    )
    .bind(key)
    .bind(value)
    .bind(Utc::now().to_rfc3339())
    .fetch_optional(pool)
    .await?;
    Ok(row.is_some())
}

/// Clear `key`; true when it stood, which is when a "resolved" line is owed.
pub async fn clear(pool: &SqlitePool, key: &str) -> Result<bool, StoreError> {
    let done = sqlx::query("DELETE FROM maidan_ops_alerts WHERE alert_key = ?")
        .bind(key)
        .execute(pool)
        .await?;
    Ok(done.rows_affected() > 0)
}

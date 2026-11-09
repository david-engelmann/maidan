//! Standing operator alerts — see the SQLite twin.

use sqlx::PgPool;

use crate::error::StoreError;

/// Stand `key` at `value`; true when it did not already stand at `value`.
pub async fn raise(pool: &PgPool, key: &str, value: &str) -> Result<bool, StoreError> {
    let row = sqlx::query(
        "INSERT INTO maidan_ops_alerts (alert_key, value, raised_at)
         VALUES ($1, $2, now())
         ON CONFLICT (alert_key) DO UPDATE
            SET value = excluded.value, raised_at = excluded.raised_at
            WHERE maidan_ops_alerts.value <> excluded.value
         RETURNING alert_key",
    )
    .bind(key)
    .bind(value)
    .fetch_optional(pool)
    .await?;
    Ok(row.is_some())
}

/// Clear `key`; true when it stood.
pub async fn clear(pool: &PgPool, key: &str) -> Result<bool, StoreError> {
    let done = sqlx::query("DELETE FROM maidan_ops_alerts WHERE alert_key = $1")
        .bind(key)
        .execute(pool)
        .await?;
    Ok(done.rows_affected() > 0)
}

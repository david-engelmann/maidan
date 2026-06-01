//! SQLite transactional outbox rows relayed to the in-process bus after commit.

use maidan_types::WorkspaceId;
use sqlx::{Row, SqlitePool};

use crate::error::StoreError;
use crate::postgres::outbox::OutboxRow;

const RELAYABLE: &str = "published_at IS NULL AND quarantined_at IS NULL";

pub async fn list_pending(pool: &SqlitePool, limit: i64) -> Result<Vec<OutboxRow>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT id, log_id, attempts
         FROM maidan_outbox
         WHERE {RELAYABLE}
         ORDER BY id ASC
         LIMIT ?"
    ))
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|row| OutboxRow {
            id: row.get("id"),
            log_id: row.get("log_id"),
            attempts: row.get("attempts"),
        })
        .collect())
}

pub async fn mark_published(pool: &SqlitePool, outbox_id: i64) -> Result<(), StoreError> {
    let updated = sqlx::query(
        "UPDATE maidan_outbox
         SET published_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE id = ? AND published_at IS NULL",
    )
    .bind(outbox_id)
    .execute(pool)
    .await?
    .rows_affected();
    if updated == 0 {
        return Err(StoreError::NotFound);
    }
    Ok(())
}

pub async fn record_attempt(pool: &SqlitePool, outbox_id: i64) -> Result<i32, StoreError> {
    let row = sqlx::query(
        "UPDATE maidan_outbox
         SET attempts = attempts + 1
         WHERE id = ?
         RETURNING attempts",
    )
    .bind(outbox_id)
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else {
        return Err(StoreError::NotFound);
    };
    Ok(row.get("attempts"))
}

pub async fn quarantine(pool: &SqlitePool, outbox_id: i64) -> Result<(), StoreError> {
    sqlx::query(
        "UPDATE maidan_outbox
         SET quarantined_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         WHERE id = ? AND quarantined_at IS NULL",
    )
    .bind(outbox_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Clears quarantine so the relay can retry; row must belong to `workspace_id`.
pub async fn replay_quarantined(
    pool: &SqlitePool,
    outbox_id: i64,
    workspace_id: WorkspaceId,
) -> Result<(), StoreError> {
    let updated = sqlx::query(
        "UPDATE maidan_outbox
         SET quarantined_at = NULL, attempts = 0
         WHERE id = ?
           AND published_at IS NULL
           AND quarantined_at IS NOT NULL
           AND log_id IN (
             SELECT id FROM maidan_events WHERE workspace_id = ?
           )",
    )
    .bind(outbox_id)
    .bind(workspace_id.0)
    .execute(pool)
    .await?
    .rows_affected();
    if updated == 0 {
        return Err(StoreError::NotFound);
    }
    Ok(())
}

pub async fn count_pending(pool: &SqlitePool) -> Result<i64, StoreError> {
    let row = sqlx::query(&format!(
        "SELECT COUNT(*) AS n FROM maidan_outbox WHERE {RELAYABLE}"
    ))
    .fetch_one(pool)
    .await?;
    Ok(row.get("n"))
}

pub async fn count_quarantined(pool: &SqlitePool) -> Result<i64, StoreError> {
    let row = sqlx::query(
        "SELECT COUNT(*) AS n
         FROM maidan_outbox
         WHERE published_at IS NULL AND quarantined_at IS NOT NULL",
    )
    .fetch_one(pool)
    .await?;
    Ok(row.get("n"))
}

pub async fn oldest_relayable_pending_age_secs(
    pool: &SqlitePool,
) -> Result<Option<f64>, StoreError> {
    let age: Option<f64> = sqlx::query_scalar(
        "SELECT (julianday('now') - julianday(MIN(created_at))) * 86400.0
         FROM maidan_outbox
         WHERE published_at IS NULL AND quarantined_at IS NULL",
    )
    .fetch_one(pool)
    .await?;
    Ok(age)
}

pub async fn enqueue_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    log_id: i64,
) -> Result<(), StoreError> {
    sqlx::query("INSERT INTO maidan_outbox (log_id) VALUES (?)")
        .bind(log_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

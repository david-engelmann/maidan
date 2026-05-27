//! Postgres transactional outbox rows relayed to `PostgresBus` after commit.

use sqlx::{PgPool, Row};

use crate::error::StoreError;

#[derive(Debug, Clone)]
pub struct OutboxRow {
    pub id: i64,
    pub log_id: i64,
    pub attempts: i32,
}

pub async fn list_pending(pool: &PgPool, limit: i64) -> Result<Vec<OutboxRow>, StoreError> {
    let rows = sqlx::query(
        "SELECT id, log_id, attempts
         FROM maidan_outbox
         WHERE published_at IS NULL
         ORDER BY id ASC
         LIMIT $1",
    )
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

pub async fn mark_published(pool: &PgPool, outbox_id: i64) -> Result<(), StoreError> {
    let updated = sqlx::query(
        "UPDATE maidan_outbox
         SET published_at = NOW()
         WHERE id = $1 AND published_at IS NULL",
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

pub async fn record_attempt(pool: &PgPool, outbox_id: i64) -> Result<(), StoreError> {
    sqlx::query(
        "UPDATE maidan_outbox
         SET attempts = attempts + 1
         WHERE id = $1",
    )
    .bind(outbox_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn count_pending(pool: &PgPool) -> Result<i64, StoreError> {
    let row =
        sqlx::query("SELECT COUNT(*)::bigint AS n FROM maidan_outbox WHERE published_at IS NULL")
            .fetch_one(pool)
            .await?;
    Ok(row.get("n"))
}

pub async fn enqueue_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    log_id: i64,
) -> Result<(), StoreError> {
    sqlx::query("INSERT INTO maidan_outbox (log_id) VALUES ($1)")
        .bind(log_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

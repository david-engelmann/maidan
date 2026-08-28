//! Durable mail outbox store (Cluster 304). SQLite twin of the Postgres module —
//! SQLite serializes writers (one connection, Cluster 277), so a select-then-update
//! in a transaction claims atomically without `FOR UPDATE SKIP LOCKED`. All
//! timestamps are store-bound rfc3339, so a plain `<=` comparison is consistent.

use chrono::{DateTime, Utc};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::StoreError;
use maidan_types::{DeadMail, MailOutbox, MailOutboxId, NewMailOutbox};

pub async fn enqueue(pool: &SqlitePool, new: NewMailOutbox) -> Result<MailOutboxId, StoreError> {
    let id = MailOutboxId::new();
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO maidan_mail_outbox
           (id, to_address, subject, body, status, attempts, next_attempt_at, created_at, updated_at)
         VALUES (?, ?, ?, ?, 'pending', 0, ?, ?, ?)",
    )
    .bind(id.0)
    .bind(&new.to_address)
    .bind(&new.subject)
    .bind(&new.body)
    .bind(&now)
    .bind(&now)
    .bind(&now)
    .execute(pool)
    .await?;
    Ok(id)
}

pub async fn claim_next_due(
    pool: &SqlitePool,
    now: DateTime<Utc>,
    lease_secs: i64,
) -> Result<Option<MailOutbox>, StoreError> {
    let now_s = now.to_rfc3339();
    let mut tx = pool.begin().await?;
    let candidate = sqlx::query(
        "SELECT id FROM maidan_mail_outbox
         WHERE status = 'pending' AND next_attempt_at <= ?
         ORDER BY next_attempt_at ASC
         LIMIT 1",
    )
    .bind(&now_s)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(cand) = candidate else {
        return Ok(None);
    };
    let id: Uuid = cand.get("id");
    let lease = (now + chrono::Duration::seconds(lease_secs)).to_rfc3339();
    let row = sqlx::query(
        "UPDATE maidan_mail_outbox
         SET attempts = attempts + 1, next_attempt_at = ?, updated_at = ?
         WHERE id = ?
         RETURNING id, to_address, subject, body, attempts",
    )
    .bind(&lease)
    .bind(&now_s)
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Some(row_to_mail(&row)))
}

pub async fn mark_delivered(pool: &SqlitePool, id: MailOutboxId) -> Result<(), StoreError> {
    let now = Utc::now().to_rfc3339();
    sqlx::query("UPDATE maidan_mail_outbox SET status = 'delivered', updated_at = ? WHERE id = ?")
        .bind(&now)
        .bind(id.0)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn mark_failed(
    pool: &SqlitePool,
    id: MailOutboxId,
    error: &str,
    retry_at: Option<DateTime<Utc>>,
) -> Result<(), StoreError> {
    let now = Utc::now().to_rfc3339();
    match retry_at {
        Some(t) => {
            sqlx::query(
                "UPDATE maidan_mail_outbox
                 SET status = 'pending', next_attempt_at = ?, last_error = ?, updated_at = ?
                 WHERE id = ?",
            )
            .bind(t.to_rfc3339())
            .bind(error)
            .bind(&now)
            .bind(id.0)
            .execute(pool)
            .await?;
        }
        None => {
            sqlx::query(
                "UPDATE maidan_mail_outbox
                 SET status = 'dead', last_error = ?, updated_at = ?
                 WHERE id = ?",
            )
            .bind(error)
            .bind(&now)
            .bind(id.0)
            .execute(pool)
            .await?;
        }
    }
    Ok(())
}

pub async fn count_dead(pool: &SqlitePool) -> Result<i64, StoreError> {
    let row = sqlx::query("SELECT COUNT(*) AS c FROM maidan_mail_outbox WHERE status = 'dead'")
        .fetch_one(pool)
        .await?;
    Ok(row.get::<i64, _>("c"))
}

pub async fn list_dead(pool: &SqlitePool, limit: i64) -> Result<Vec<DeadMail>, StoreError> {
    let rows = sqlx::query(
        "SELECT id, to_address, subject, attempts, last_error, updated_at
         FROM maidan_mail_outbox
         WHERE status = 'dead'
         ORDER BY updated_at DESC
         LIMIT ?",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_dead).collect())
}

pub async fn requeue_dead(pool: &SqlitePool, id: MailOutboxId) -> Result<bool, StoreError> {
    let now = Utc::now().to_rfc3339();
    let res = sqlx::query(
        "UPDATE maidan_mail_outbox
         SET status = 'pending', attempts = 0, next_attempt_at = ?, updated_at = ?
         WHERE id = ? AND status = 'dead'",
    )
    .bind(&now)
    .bind(&now)
    .bind(id.0)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

fn row_to_dead(row: &sqlx::sqlite::SqliteRow) -> DeadMail {
    DeadMail {
        id: MailOutboxId(row.get("id")),
        to_address: row.get("to_address"),
        subject: row.get("subject"),
        attempts: row.get("attempts"),
        last_error: row.get("last_error"),
        updated_at: row.get("updated_at"),
    }
}

fn row_to_mail(row: &sqlx::sqlite::SqliteRow) -> MailOutbox {
    MailOutbox {
        id: MailOutboxId(row.get("id")),
        to_address: row.get("to_address"),
        subject: row.get("subject"),
        body: row.get("body"),
        attempts: row.get("attempts"),
    }
}

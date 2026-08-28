//! Durable mail outbox store (Cluster 304): enqueue notification emails and let a
//! retry/backoff worker claim + deliver them, instead of a best-effort send with
//! no retry. See the SQLite twin.

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};

use crate::StoreError;
use maidan_types::{DeadMail, MailOutbox, MailOutboxId, NewMailOutbox};

/// Enqueue an email for durable delivery: `pending`, due now.
pub async fn enqueue(pool: &PgPool, new: NewMailOutbox) -> Result<MailOutboxId, StoreError> {
    let id = MailOutboxId::new();
    sqlx::query(
        "INSERT INTO maidan_mail_outbox
           (id, to_address, subject, body, status, attempts, next_attempt_at, created_at, updated_at)
         VALUES ($1, $2, $3, $4, 'pending', 0, now(), now(), now())",
    )
    .bind(id.0)
    .bind(&new.to_address)
    .bind(&new.subject)
    .bind(&new.body)
    .execute(pool)
    .await?;
    Ok(id)
}

/// Atomically claim the oldest due pending row: lease it forward
/// (`next_attempt_at = now + lease_secs`) and bump `attempts`, so a worker that
/// crashes mid-send releases the row after the lease (at-least-once — a duplicate
/// email is low-harm, matching the digest polarity). `FOR UPDATE SKIP LOCKED` lets
/// concurrent replicas claim distinct rows.
pub async fn claim_next_due(
    pool: &PgPool,
    now: DateTime<Utc>,
    lease_secs: i64,
) -> Result<Option<MailOutbox>, StoreError> {
    let row = sqlx::query(
        "WITH due AS (
             SELECT id FROM maidan_mail_outbox
             WHERE status = 'pending' AND next_attempt_at <= $1
             ORDER BY next_attempt_at ASC
             LIMIT 1
             FOR UPDATE SKIP LOCKED
         )
         UPDATE maidan_mail_outbox m
         SET attempts = m.attempts + 1,
             next_attempt_at = $1 + make_interval(secs => $2),
             updated_at = now()
         FROM due
         WHERE m.id = due.id
         RETURNING m.id, m.to_address, m.subject, m.body, m.attempts",
    )
    .bind(now)
    .bind(lease_secs as f64)
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(row_to_mail))
}

/// Mark a claimed entry delivered.
pub async fn mark_delivered(pool: &PgPool, id: MailOutboxId) -> Result<(), StoreError> {
    sqlx::query(
        "UPDATE maidan_mail_outbox SET status = 'delivered', updated_at = now() WHERE id = $1",
    )
    .bind(id.0)
    .execute(pool)
    .await?;
    Ok(())
}

/// On a failed send: `retry_at = Some(t)` reschedules (stays `pending`,
/// `next_attempt_at = t`); `None` dead-letters (`status = 'dead'`). The worker
/// decides based on the attempt count.
pub async fn mark_failed(
    pool: &PgPool,
    id: MailOutboxId,
    error: &str,
    retry_at: Option<DateTime<Utc>>,
) -> Result<(), StoreError> {
    match retry_at {
        Some(t) => {
            sqlx::query(
                "UPDATE maidan_mail_outbox
                 SET status = 'pending', next_attempt_at = $2, last_error = $3, updated_at = now()
                 WHERE id = $1",
            )
            .bind(id.0)
            .bind(t)
            .bind(error)
            .execute(pool)
            .await?;
        }
        None => {
            sqlx::query(
                "UPDATE maidan_mail_outbox
                 SET status = 'dead', last_error = $2, updated_at = now()
                 WHERE id = $1",
            )
            .bind(id.0)
            .bind(error)
            .execute(pool)
            .await?;
        }
    }
    Ok(())
}

/// Count dead-lettered entries (DLQ depth) — for metrics / ops.
pub async fn count_dead(pool: &PgPool) -> Result<i64, StoreError> {
    let row = sqlx::query("SELECT COUNT(*) AS c FROM maidan_mail_outbox WHERE status = 'dead'")
        .fetch_one(pool)
        .await?;
    Ok(row.get::<i64, _>("c"))
}

/// List dead-lettered entries, newest-updated first (the operator DLQ view).
pub async fn list_dead(pool: &PgPool, limit: i64) -> Result<Vec<DeadMail>, StoreError> {
    let rows = sqlx::query(
        "SELECT id, to_address, subject, attempts, last_error, updated_at
         FROM maidan_mail_outbox
         WHERE status = 'dead'
         ORDER BY updated_at DESC
         LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_dead).collect())
}

/// Requeue a dead entry for a fresh delivery attempt: `pending`, due now,
/// `attempts` reset. Returns whether a dead row was actually requeued.
pub async fn requeue_dead(pool: &PgPool, id: MailOutboxId) -> Result<bool, StoreError> {
    let res = sqlx::query(
        "UPDATE maidan_mail_outbox
         SET status = 'pending', attempts = 0, next_attempt_at = now(), updated_at = now()
         WHERE id = $1 AND status = 'dead'",
    )
    .bind(id.0)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

fn row_to_dead(row: &sqlx::postgres::PgRow) -> DeadMail {
    DeadMail {
        id: MailOutboxId(row.get("id")),
        to_address: row.get("to_address"),
        subject: row.get("subject"),
        attempts: row.get("attempts"),
        last_error: row.get("last_error"),
        updated_at: row.get("updated_at"),
    }
}

fn row_to_mail(row: &sqlx::postgres::PgRow) -> MailOutbox {
    MailOutbox {
        id: MailOutboxId(row.get("id")),
        to_address: row.get("to_address"),
        subject: row.get("subject"),
        body: row.get("body"),
        attempts: row.get("attempts"),
    }
}

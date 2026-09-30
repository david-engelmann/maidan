//! Durable mail outbox store. SQLite twin of the Postgres module — SQLite
//! serializes writers (one connection), so a select-then-update in a
//! transaction claims atomically without `FOR UPDATE SKIP LOCKED`. All
//! timestamps are store-bound rfc3339, so a plain `<=` comparison is
//! consistent.

use chrono::{DateTime, Utc};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::StoreError;
use maidan_types::{DeadMail, MailOutbox, MailOutboxId, NewMailOutbox, WorkspaceId};

/// One statement, so it is atomic against a shred: a mail about a message event
/// is linked to the message's content key, and nothing is queued (`None`) when
/// that key is already shredded.
pub async fn enqueue(
    pool: &SqlitePool,
    new: NewMailOutbox,
) -> Result<Option<MailOutboxId>, StoreError> {
    let id = MailOutboxId::new();
    let now = Utc::now().to_rfc3339();
    let inserted = sqlx::query(
        "INSERT INTO maidan_mail_outbox
           (id, workspace_id, content_key_id, to_address, subject, body, status, attempts,
            next_attempt_at, created_at, updated_at)
         SELECT ?1, ?2, (SELECT content_key_id FROM maidan_events WHERE id = ?3),
                ?4, ?5, ?6, 'pending', 0, ?7, ?7, ?7
         WHERE NOT EXISTS (
             SELECT 1 FROM maidan_events e
             JOIN maidan_content_keys k ON k.id = e.content_key_id
             WHERE e.id = ?3 AND k.shredded_at IS NOT NULL
         )",
    )
    .bind(id.0)
    .bind(new.workspace_id.map(|w| w.0))
    .bind(new.source_log_id)
    .bind(&new.to_address)
    .bind(&new.subject)
    .bind(&new.body)
    .bind(&now)
    .execute(pool)
    .await?
    .rows_affected();
    Ok((inserted > 0).then_some(id))
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

/// Hand a claimed entry back unsent: give back the claim's attempt and make it
/// due at `until`. Only a `pending` row moves, so a row that was delivered or
/// dead-lettered meanwhile is left alone.
pub async fn defer(
    pool: &SqlitePool,
    id: MailOutboxId,
    until: DateTime<Utc>,
) -> Result<(), StoreError> {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "UPDATE maidan_mail_outbox
         SET attempts = MAX(attempts - 1, 0), next_attempt_at = ?, updated_at = ?
         WHERE id = ? AND status = 'pending'",
    )
    .bind(until.to_rfc3339())
    .bind(&now)
    .bind(id.0)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn count_dead(pool: &SqlitePool) -> Result<i64, StoreError> {
    let row = sqlx::query("SELECT COUNT(*) AS c FROM maidan_mail_outbox WHERE status = 'dead'")
        .fetch_one(pool)
        .await?;
    Ok(row.get::<i64, _>("c"))
}

/// Dead-lettered mail for the operator DLQ, scoped like the Postgres twin.
/// `None` is the `operator:global` view and the only way to see a row whose
/// workspace is `NULL`.
pub async fn list_dead(
    pool: &SqlitePool,
    scope: Option<WorkspaceId>,
    limit: i64,
) -> Result<Vec<DeadMail>, StoreError> {
    let scope_id = scope.map(|w| w.0);
    let rows = sqlx::query(
        "SELECT id, workspace_id, to_address, subject, attempts, last_error, updated_at
         FROM maidan_mail_outbox
         WHERE status = 'dead'
           AND (? IS NULL OR workspace_id = ?)
         ORDER BY updated_at DESC
         LIMIT ?",
    )
    .bind(scope_id)
    .bind(scope_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_dead).collect())
}

/// Requeue a dead entry, scoped like [`list_dead`].
pub async fn requeue_dead(
    pool: &SqlitePool,
    scope: Option<WorkspaceId>,
    id: MailOutboxId,
) -> Result<bool, StoreError> {
    let now = Utc::now().to_rfc3339();
    let scope_id = scope.map(|w| w.0);
    let res = sqlx::query(
        "UPDATE maidan_mail_outbox
         SET status = 'pending', attempts = 0, next_attempt_at = ?, updated_at = ?
         WHERE id = ? AND status = 'dead'
           AND (? IS NULL OR workspace_id = ?)",
    )
    .bind(&now)
    .bind(&now)
    .bind(id.0)
    .bind(scope_id)
    .bind(scope_id)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

fn row_to_dead(row: &sqlx::sqlite::SqliteRow) -> DeadMail {
    DeadMail {
        id: MailOutboxId(row.get("id")),
        workspace_id: row
            .get::<Option<uuid::Uuid>, _>("workspace_id")
            .map(WorkspaceId),
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

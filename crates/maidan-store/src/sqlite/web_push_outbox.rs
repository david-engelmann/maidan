//! Durable web push outbox. SQLite twin of the Postgres module.

use chrono::{DateTime, Utc};
use maidan_types::{
    MemberId, NewWebPushOutbox, PushSubscriptionId, WebPushOutbox, WebPushOutboxId,
};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::StoreError;

pub async fn enqueue(
    pool: &SqlitePool,
    new: NewWebPushOutbox,
) -> Result<WebPushOutboxId, StoreError> {
    let id = WebPushOutboxId::new();
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO maidan_web_push_outbox
           (id, member_id, subscription_id, payload, status, attempts, next_attempt_at,
            last_error, created_at, updated_at)
         VALUES (?, ?, ?, ?, 'pending', ?, ?, ?, ?, ?)",
    )
    .bind(id.0)
    .bind(new.member_id.0)
    .bind(new.subscription_id.0)
    .bind(&new.payload)
    .bind(new.attempts)
    .bind(new.next_attempt_at.to_rfc3339())
    .bind(&new.last_error)
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
) -> Result<Option<WebPushOutbox>, StoreError> {
    let now_s = now.to_rfc3339();
    let mut tx = pool.begin().await?;
    let candidate = sqlx::query(
        "SELECT id FROM maidan_web_push_outbox
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
        "UPDATE maidan_web_push_outbox
         SET attempts = attempts + 1, next_attempt_at = ?, updated_at = ?
         WHERE id = ?
         RETURNING id, member_id, subscription_id, payload, attempts",
    )
    .bind(&lease)
    .bind(&now_s)
    .bind(id)
    .fetch_one(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(Some(row_to_push(&row)))
}

pub async fn mark_delivered(pool: &SqlitePool, id: WebPushOutboxId) -> Result<(), StoreError> {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "UPDATE maidan_web_push_outbox SET status = 'delivered', updated_at = ? WHERE id = ?",
    )
    .bind(&now)
    .bind(id.0)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn mark_failed(
    pool: &SqlitePool,
    id: WebPushOutboxId,
    error: &str,
    retry_at: Option<DateTime<Utc>>,
) -> Result<(), StoreError> {
    let now = Utc::now().to_rfc3339();
    match retry_at {
        Some(t) => {
            sqlx::query(
                "UPDATE maidan_web_push_outbox
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
                "UPDATE maidan_web_push_outbox
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

pub async fn defer(
    pool: &SqlitePool,
    id: WebPushOutboxId,
    until: DateTime<Utc>,
) -> Result<(), StoreError> {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "UPDATE maidan_web_push_outbox
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

fn row_to_push(row: &sqlx::sqlite::SqliteRow) -> WebPushOutbox {
    WebPushOutbox {
        id: WebPushOutboxId(row.get("id")),
        member_id: MemberId(row.get("member_id")),
        subscription_id: PushSubscriptionId(row.get("subscription_id")),
        payload: row.get("payload"),
        attempts: row.get("attempts"),
    }
}

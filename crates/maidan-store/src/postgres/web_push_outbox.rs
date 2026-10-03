//! Durable web push outbox. A failed send is claimed and retried; a crash
//! mid-send releases the row after the lease.

use chrono::{DateTime, Utc};
use maidan_types::{
    MemberId, NewWebPushOutbox, PushSubscriptionId, WebPushOutbox, WebPushOutboxId,
};
use sqlx::{PgPool, Row};

use crate::StoreError;

pub async fn enqueue(pool: &PgPool, new: NewWebPushOutbox) -> Result<WebPushOutboxId, StoreError> {
    let id = WebPushOutboxId::new();
    sqlx::query(
        "INSERT INTO maidan_web_push_outbox
           (id, member_id, subscription_id, payload, status, attempts, next_attempt_at,
            last_error, created_at, updated_at)
         VALUES ($1, $2, $3, $4, 'pending', $5, $6, $7, now(), now())",
    )
    .bind(id.0)
    .bind(new.member_id.0)
    .bind(new.subscription_id.0)
    .bind(&new.payload)
    .bind(new.attempts)
    .bind(new.next_attempt_at)
    .bind(&new.last_error)
    .execute(pool)
    .await?;
    Ok(id)
}

pub async fn claim_next_due(
    pool: &PgPool,
    now: DateTime<Utc>,
    lease_secs: i64,
) -> Result<Option<WebPushOutbox>, StoreError> {
    let row = sqlx::query(
        "WITH due AS (
             SELECT id FROM maidan_web_push_outbox
             WHERE status = 'pending' AND next_attempt_at <= $1
             ORDER BY next_attempt_at ASC
             LIMIT 1
             FOR UPDATE SKIP LOCKED
         )
         UPDATE maidan_web_push_outbox m
         SET attempts = m.attempts + 1,
             next_attempt_at = $1 + make_interval(secs => $2),
             updated_at = now()
         FROM due
         WHERE m.id = due.id
         RETURNING m.id, m.member_id, m.subscription_id, m.payload, m.attempts",
    )
    .bind(now)
    .bind(lease_secs as f64)
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(row_to_push))
}

pub async fn mark_delivered(pool: &PgPool, id: WebPushOutboxId) -> Result<(), StoreError> {
    sqlx::query(
        "UPDATE maidan_web_push_outbox SET status = 'delivered', updated_at = now() WHERE id = $1",
    )
    .bind(id.0)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn mark_failed(
    pool: &PgPool,
    id: WebPushOutboxId,
    error: &str,
    retry_at: Option<DateTime<Utc>>,
) -> Result<(), StoreError> {
    match retry_at {
        Some(t) => {
            sqlx::query(
                "UPDATE maidan_web_push_outbox
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
                "UPDATE maidan_web_push_outbox
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

pub async fn defer(
    pool: &PgPool,
    id: WebPushOutboxId,
    until: DateTime<Utc>,
) -> Result<(), StoreError> {
    sqlx::query(
        "UPDATE maidan_web_push_outbox
         SET attempts = GREATEST(attempts - 1, 0), next_attempt_at = $2, updated_at = now()
         WHERE id = $1 AND status = 'pending'",
    )
    .bind(id.0)
    .bind(until)
    .execute(pool)
    .await?;
    Ok(())
}

fn row_to_push(row: &sqlx::postgres::PgRow) -> WebPushOutbox {
    WebPushOutbox {
        id: WebPushOutboxId(row.get("id")),
        member_id: MemberId(row.get("member_id")),
        subscription_id: PushSubscriptionId(row.get("subscription_id")),
        payload: row.get("payload"),
        attempts: row.get("attempts"),
    }
}

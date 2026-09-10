//! Thread wait-timer queries (Cluster 364, G2/G4): the `maidan_thread_waits` side
//! table + the sweeper's atomic fire-once claim (`FOR UPDATE SKIP LOCKED`).

use chrono::{DateTime, Utc};
use maidan_types::{EscalationPolicy, MemberId, ThreadId, ThreadWait};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::StoreError;

fn row_to_wait(row: &sqlx::postgres::PgRow) -> ThreadWait {
    let on_timeout: String = row.get("on_timeout");
    ThreadWait {
        thread_id: ThreadId(row.get::<Uuid, _>("thread_id")),
        wait_until: row.get::<DateTime<Utc>, _>("wait_until"),
        on_timeout: EscalationPolicy::parse(&on_timeout).unwrap_or_default(),
        reason: row.get::<Option<String>, _>("reason"),
        created_by: MemberId(row.get::<Uuid, _>("created_by")),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
        fired_at: row.get::<Option<DateTime<Utc>>, _>("fired_at"),
    }
}

const COLS: &str = "thread_id, wait_until, on_timeout, reason, created_by, created_at, fired_at";

pub async fn set(
    pool: &PgPool,
    thread_id: ThreadId,
    wait_until: DateTime<Utc>,
    on_timeout: EscalationPolicy,
    reason: Option<&str>,
    created_by: MemberId,
) -> Result<ThreadWait, StoreError> {
    let row = sqlx::query(&format!(
        "INSERT INTO maidan_thread_waits
             (thread_id, wait_until, on_timeout, reason, created_by)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (thread_id) DO UPDATE SET
             wait_until = EXCLUDED.wait_until,
             on_timeout = EXCLUDED.on_timeout,
             reason = EXCLUDED.reason,
             created_by = EXCLUDED.created_by,
             fired_at = NULL
         RETURNING {COLS}"
    ))
    .bind(thread_id.0)
    .bind(wait_until)
    .bind(on_timeout.as_str())
    .bind(reason)
    .bind(created_by.0)
    .fetch_one(pool)
    .await?;
    Ok(row_to_wait(&row))
}

pub async fn cancel(pool: &PgPool, thread_id: ThreadId) -> Result<bool, StoreError> {
    let done = sqlx::query("DELETE FROM maidan_thread_waits WHERE thread_id = $1")
        .bind(thread_id.0)
        .execute(pool)
        .await?;
    Ok(done.rows_affected() > 0)
}

pub async fn get(pool: &PgPool, thread_id: ThreadId) -> Result<Option<ThreadWait>, StoreError> {
    let row = sqlx::query(&format!(
        "SELECT {COLS} FROM maidan_thread_waits WHERE thread_id = $1"
    ))
    .bind(thread_id.0)
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(row_to_wait))
}

pub async fn claim_next_due(
    pool: &PgPool,
    now: DateTime<Utc>,
) -> Result<Option<ThreadWait>, StoreError> {
    let row = sqlx::query(&format!(
        "UPDATE maidan_thread_waits SET fired_at = $1
         WHERE thread_id = (
             SELECT thread_id FROM maidan_thread_waits
             WHERE fired_at IS NULL AND wait_until <= $1
             ORDER BY wait_until ASC, thread_id ASC
             LIMIT 1
             FOR UPDATE SKIP LOCKED
         )
         RETURNING {COLS}"
    ))
    .bind(now)
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(row_to_wait))
}

//! Thread dispatch-park queries (Cluster 363, G3): the `maidan_thread_unclaimable`
//! side table. Presence = parked from dispatch; absence = claimable.

use chrono::{DateTime, Utc};
use maidan_types::{ChannelId, MemberId, ThreadId, ThreadUnclaimable};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::StoreError;

fn row_to_unclaimable(row: &sqlx::postgres::PgRow) -> ThreadUnclaimable {
    ThreadUnclaimable {
        thread_id: ThreadId(row.get::<Uuid, _>("thread_id")),
        reason: row.get::<String, _>("reason"),
        marked_by: MemberId(row.get::<Uuid, _>("marked_by")),
        marked_at: row.get::<DateTime<Utc>, _>("marked_at"),
    }
}

pub async fn mark(
    pool: &PgPool,
    thread_id: ThreadId,
    reason: &str,
    marked_by: MemberId,
) -> Result<ThreadUnclaimable, StoreError> {
    let row = sqlx::query(
        "INSERT INTO maidan_thread_unclaimable (thread_id, reason, marked_by)
         VALUES ($1, $2, $3)
         ON CONFLICT (thread_id) DO UPDATE SET
             reason = EXCLUDED.reason, marked_by = EXCLUDED.marked_by, marked_at = NOW()
         RETURNING thread_id, reason, marked_by, marked_at",
    )
    .bind(thread_id.0)
    .bind(reason)
    .bind(marked_by.0)
    .fetch_one(pool)
    .await?;
    Ok(row_to_unclaimable(&row))
}

pub async fn clear(pool: &PgPool, thread_id: ThreadId) -> Result<bool, StoreError> {
    let done = sqlx::query("DELETE FROM maidan_thread_unclaimable WHERE thread_id = $1")
        .bind(thread_id.0)
        .execute(pool)
        .await?;
    Ok(done.rows_affected() > 0)
}

pub async fn get(
    pool: &PgPool,
    thread_id: ThreadId,
) -> Result<Option<ThreadUnclaimable>, StoreError> {
    let row = sqlx::query(
        "SELECT thread_id, reason, marked_by, marked_at
         FROM maidan_thread_unclaimable WHERE thread_id = $1",
    )
    .bind(thread_id.0)
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(row_to_unclaimable))
}

pub async fn list_for_channel(
    pool: &PgPool,
    channel_id: ChannelId,
) -> Result<Vec<ThreadUnclaimable>, StoreError> {
    let rows = sqlx::query(
        "SELECT u.thread_id, u.reason, u.marked_by, u.marked_at
         FROM maidan_thread_unclaimable u
         JOIN maidan_threads t ON t.id = u.thread_id
         WHERE t.channel_id = $1 AND t.tombstoned_at IS NULL
         ORDER BY u.marked_at DESC, u.thread_id",
    )
    .bind(channel_id.0)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_unclaimable).collect())
}

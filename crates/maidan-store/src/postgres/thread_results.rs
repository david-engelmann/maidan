use chrono::{DateTime, Utc};
use maidan_types::{ChannelClosedResult, ChannelId, MemberId, ThreadId, ThreadResult, ThreadState};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::StoreError;

/// Set (upsert) a task's structured result (Cluster 234) — see the SQLite twin.
/// `result` binds directly to the JSONB column.
pub async fn set(
    pool: &PgPool,
    thread_id: ThreadId,
    produced_by: MemberId,
    result: &serde_json::Value,
) -> Result<ThreadResult, StoreError> {
    let row = sqlx::query(
        "INSERT INTO maidan_thread_results (thread_id, result, produced_by)
         VALUES ($1, $2, $3)
         ON CONFLICT (thread_id) DO UPDATE SET
             result = excluded.result,
             produced_by = excluded.produced_by,
             produced_at = now()
         RETURNING thread_id, result, produced_by, produced_at",
    )
    .bind(thread_id.0)
    .bind(result)
    .bind(produced_by.0)
    .fetch_one(pool)
    .await?;
    Ok(row_to_result(&row))
}

pub async fn get(pool: &PgPool, thread_id: ThreadId) -> Result<Option<ThreadResult>, StoreError> {
    let row = sqlx::query(
        "SELECT thread_id, result, produced_by, produced_at
         FROM maidan_thread_results WHERE thread_id = $1",
    )
    .bind(thread_id.0)
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(row_to_result))
}

fn row_to_result(row: &sqlx::postgres::PgRow) -> ThreadResult {
    ThreadResult {
        thread_id: ThreadId(row.get::<Uuid, _>("thread_id")),
        result: row.get::<serde_json::Value, _>("result"),
        produced_by: MemberId(row.get::<Uuid, _>("produced_by")),
        produced_at: row.get::<DateTime<Utc>, _>("produced_at"),
    }
}

/// Closed/archived results in a channel (Cluster 382, Wave 2 #24) — see the
/// SQLite twin. `exclude_thread_id` is the claimer's own thread; `limit` is
/// clamped `1..=50`.
pub async fn list_closed_in_channel(
    pool: &PgPool,
    channel_id: ChannelId,
    exclude_thread_id: Option<ThreadId>,
    limit: i64,
) -> Result<Vec<ChannelClosedResult>, StoreError> {
    let limit = limit.clamp(1, 50);
    let rows = sqlx::query(
        "SELECT tr.thread_id AS thread_id, t.title AS title, t.state AS state,
                tr.result AS result, tr.produced_by AS produced_by,
                tr.produced_at AS produced_at
         FROM maidan_thread_results tr
         JOIN maidan_threads t ON t.id = tr.thread_id
         WHERE t.channel_id = $1
           AND t.state IN ('closed', 'archived')
           AND t.tombstoned_at IS NULL
           AND ($2::uuid IS NULL OR t.id <> $2)
         ORDER BY tr.produced_at DESC
         LIMIT $3",
    )
    .bind(channel_id.0)
    .bind(exclude_thread_id.map(|id| id.0))
    .bind(limit)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_closed).collect()
}

fn row_to_closed(row: &sqlx::postgres::PgRow) -> Result<ChannelClosedResult, StoreError> {
    let state_str: String = row.get("state");
    let state = ThreadState::parse(&state_str)
        .ok_or_else(|| StoreError::InvalidInput(format!("unknown thread state: {state_str}")))?;
    Ok(ChannelClosedResult {
        thread_id: ThreadId(row.get::<Uuid, _>("thread_id")),
        title: row.get("title"),
        state,
        result: row.get::<serde_json::Value, _>("result"),
        produced_by: MemberId(row.get::<Uuid, _>("produced_by")),
        produced_at: row.get::<DateTime<Utc>, _>("produced_at"),
    })
}

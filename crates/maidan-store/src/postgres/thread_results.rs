use chrono::{DateTime, Utc};
use maidan_types::{MemberId, ThreadId, ThreadResult};
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

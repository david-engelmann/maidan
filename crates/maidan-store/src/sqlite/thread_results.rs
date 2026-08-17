use chrono::{DateTime, Utc};
use maidan_types::{MemberId, ThreadId, ThreadResult};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;

/// Set (upsert) a task's structured result (Cluster 234). A re-set overwrites the
/// prior result. JSON is stored as TEXT in SQLite.
pub async fn set(
    pool: &SqlitePool,
    thread_id: ThreadId,
    produced_by: MemberId,
    result: &serde_json::Value,
) -> Result<ThreadResult, StoreError> {
    let result_text = serde_json::to_string(result)?;
    let now = Utc::now().to_rfc3339();
    let row = sqlx::query(
        "INSERT INTO maidan_thread_results (thread_id, result, produced_by, produced_at)
         VALUES (?, ?, ?, ?)
         ON CONFLICT (thread_id) DO UPDATE SET
             result = excluded.result,
             produced_by = excluded.produced_by,
             produced_at = excluded.produced_at
         RETURNING thread_id, result, produced_by, produced_at",
    )
    .bind(thread_id.0)
    .bind(&result_text)
    .bind(produced_by.0)
    .bind(&now)
    .fetch_one(pool)
    .await?;
    row_to_result(&row)
}

/// A task's result, or `None` if none has been produced (Cluster 234).
pub async fn get(
    pool: &SqlitePool,
    thread_id: ThreadId,
) -> Result<Option<ThreadResult>, StoreError> {
    let row = sqlx::query(
        "SELECT thread_id, result, produced_by, produced_at
         FROM maidan_thread_results WHERE thread_id = ?",
    )
    .bind(thread_id.0)
    .fetch_optional(pool)
    .await?;
    row.as_ref().map(row_to_result).transpose()
}

fn row_to_result(row: &sqlx::sqlite::SqliteRow) -> Result<ThreadResult, StoreError> {
    let result_text: String = row.get("result");
    Ok(ThreadResult {
        thread_id: ThreadId(row.get::<Uuid, _>("thread_id")),
        result: serde_json::from_str(&result_text)?,
        produced_by: MemberId(row.get::<Uuid, _>("produced_by")),
        produced_at: row.get::<DateTime<Utc>, _>("produced_at"),
    })
}

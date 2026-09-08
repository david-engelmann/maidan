use chrono::{DateTime, Utc};
use maidan_types::{MemberId, ThreadId, ThreadSteer};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;

/// Set (upsert) a thread's persisted steer (Cluster 355, W1). Latest wins; one
/// steer per thread.
pub async fn set(
    pool: &SqlitePool,
    thread_id: ThreadId,
    steered_by: MemberId,
    steer: &str,
) -> Result<ThreadSteer, StoreError> {
    let now = Utc::now().to_rfc3339();
    let row = sqlx::query(
        "INSERT INTO maidan_thread_steer (thread_id, steer, steered_by, steered_at)
         VALUES (?, ?, ?, ?)
         ON CONFLICT (thread_id) DO UPDATE SET
             steer = excluded.steer,
             steered_by = excluded.steered_by,
             steered_at = excluded.steered_at
         RETURNING thread_id, steer, steered_by, steered_at",
    )
    .bind(thread_id.0)
    .bind(steer)
    .bind(steered_by.0)
    .bind(&now)
    .fetch_one(pool)
    .await?;
    Ok(row_to_steer(&row))
}

/// A thread's steer, or `None` if none has been set (Cluster 355).
pub async fn get(
    pool: &SqlitePool,
    thread_id: ThreadId,
) -> Result<Option<ThreadSteer>, StoreError> {
    let row = sqlx::query(
        "SELECT thread_id, steer, steered_by, steered_at
         FROM maidan_thread_steer WHERE thread_id = ?",
    )
    .bind(thread_id.0)
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(row_to_steer))
}

fn row_to_steer(row: &sqlx::sqlite::SqliteRow) -> ThreadSteer {
    ThreadSteer {
        thread_id: ThreadId(row.get::<Uuid, _>("thread_id")),
        steer: row.get::<String, _>("steer"),
        steered_by: MemberId(row.get::<Uuid, _>("steered_by")),
        steered_at: row.get::<DateTime<Utc>, _>("steered_at"),
    }
}

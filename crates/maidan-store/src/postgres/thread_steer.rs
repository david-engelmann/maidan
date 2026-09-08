use chrono::{DateTime, Utc};
use maidan_types::{MemberId, ThreadId, ThreadSteer};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::StoreError;

/// Set (upsert) a thread's persisted steer (Cluster 355, W1) — see the SQLite
/// twin. Latest wins; one steer per thread.
pub async fn set(
    pool: &PgPool,
    thread_id: ThreadId,
    steered_by: MemberId,
    steer: &str,
) -> Result<ThreadSteer, StoreError> {
    let row = sqlx::query(
        "INSERT INTO maidan_thread_steer (thread_id, steer, steered_by)
         VALUES ($1, $2, $3)
         ON CONFLICT (thread_id) DO UPDATE SET
             steer = excluded.steer,
             steered_by = excluded.steered_by,
             steered_at = now()
         RETURNING thread_id, steer, steered_by, steered_at",
    )
    .bind(thread_id.0)
    .bind(steer)
    .bind(steered_by.0)
    .fetch_one(pool)
    .await?;
    Ok(row_to_steer(&row))
}

pub async fn get(pool: &PgPool, thread_id: ThreadId) -> Result<Option<ThreadSteer>, StoreError> {
    let row = sqlx::query(
        "SELECT thread_id, steer, steered_by, steered_at
         FROM maidan_thread_steer WHERE thread_id = $1",
    )
    .bind(thread_id.0)
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(row_to_steer))
}

fn row_to_steer(row: &sqlx::postgres::PgRow) -> ThreadSteer {
    ThreadSteer {
        thread_id: ThreadId(row.get::<Uuid, _>("thread_id")),
        steer: row.get::<String, _>("steer"),
        steered_by: MemberId(row.get::<Uuid, _>("steered_by")),
        steered_at: row.get::<DateTime<Utc>, _>("steered_at"),
    }
}

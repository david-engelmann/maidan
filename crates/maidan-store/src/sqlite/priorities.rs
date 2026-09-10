//! Thread dispatch-priority queries (Cluster 365, G3 fair dispatch): the
//! `maidan_thread_priorities` side table. `claim_next`'s aged-rank ordering
//! (`threads.rs`) LEFT JOINs this; a missing row means the default priority 0.

use chrono::{DateTime, Utc};
use maidan_types::{MemberId, ThreadId, ThreadPriority};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;

fn row_to_priority(row: &sqlx::sqlite::SqliteRow) -> ThreadPriority {
    ThreadPriority {
        thread_id: ThreadId(row.get::<Uuid, _>("thread_id")),
        priority: row.get::<i64, _>("priority"),
        set_by: MemberId(row.get::<Uuid, _>("set_by")),
        set_at: row.get::<DateTime<Utc>, _>("set_at"),
    }
}

const COLS: &str = "thread_id, priority, set_by, set_at";

pub async fn set(
    pool: &SqlitePool,
    thread_id: ThreadId,
    priority: i64,
    set_by: MemberId,
) -> Result<ThreadPriority, StoreError> {
    let now = Utc::now().to_rfc3339();
    let row = sqlx::query(&format!(
        "INSERT INTO maidan_thread_priorities (thread_id, priority, set_by, set_at)
         VALUES (?, ?, ?, ?)
         ON CONFLICT (thread_id) DO UPDATE SET
             priority = excluded.priority,
             set_by = excluded.set_by,
             set_at = excluded.set_at
         RETURNING {COLS}"
    ))
    .bind(thread_id.0)
    .bind(priority)
    .bind(set_by.0)
    .bind(&now)
    .fetch_one(pool)
    .await?;
    Ok(row_to_priority(&row))
}

pub async fn get(
    pool: &SqlitePool,
    thread_id: ThreadId,
) -> Result<Option<ThreadPriority>, StoreError> {
    let row = sqlx::query(&format!(
        "SELECT {COLS} FROM maidan_thread_priorities WHERE thread_id = ?"
    ))
    .bind(thread_id.0)
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(row_to_priority))
}

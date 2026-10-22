//! Run lineage — see the Postgres twin.
//!
//! `parent_run_id` is the producer's `run_id` as-is. Nested occupancy
//! attributes every open workspace thread that shares it. F7 mute is a
//! different table and is not consulted.

use chrono::{DateTime, Utc};
use maidan_types::{
    normalize_parent_run_id, RunOccupancy, Thread, ThreadId, ThreadLineage, WorkspaceId,
};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;
use crate::sqlite::threads::row_to_thread;

const THREAD_COLS: &str =
    "t.id, t.channel_id, t.parent_thread_id, t.title, t.description, t.state, \
     t.created_at, t.updated_at, t.tombstoned_at, t.assignee_id, \
     t.assignment_expires_at, t.claim_lease_id, t.work_started_at, t.owner_id, t.created_by";

fn normalize(parent_run_id: &str) -> Result<&str, StoreError> {
    normalize_parent_run_id(parent_run_id).ok_or_else(|| {
        StoreError::InvalidInput(
            "parent_run_id must be a non-empty producer run id (max 256 bytes)".into(),
        )
    })
}

pub async fn set(
    pool: &SqlitePool,
    thread_id: ThreadId,
    parent_run_id: &str,
) -> Result<ThreadLineage, StoreError> {
    let parent_run_id = normalize(parent_run_id)?;
    let now = Utc::now().to_rfc3339();
    let row = sqlx::query(
        "INSERT INTO maidan_thread_lineage (thread_id, parent_run_id, set_at)
         VALUES (?, ?, ?)
         ON CONFLICT (thread_id) DO UPDATE SET
             parent_run_id = excluded.parent_run_id,
             set_at = excluded.set_at
         RETURNING thread_id, parent_run_id, set_at",
    )
    .bind(thread_id.0)
    .bind(parent_run_id)
    .bind(&now)
    .fetch_one(pool)
    .await?;
    row_to_lineage(&row)
}

pub async fn get(
    pool: &SqlitePool,
    thread_id: ThreadId,
) -> Result<Option<ThreadLineage>, StoreError> {
    let row = sqlx::query(
        "SELECT thread_id, parent_run_id, set_at
         FROM maidan_thread_lineage WHERE thread_id = ?",
    )
    .bind(thread_id.0)
    .fetch_optional(pool)
    .await?;
    row.as_ref().map(row_to_lineage).transpose()
}

pub async fn clear(pool: &SqlitePool, thread_id: ThreadId) -> Result<bool, StoreError> {
    let result = sqlx::query("DELETE FROM maidan_thread_lineage WHERE thread_id = ?")
        .bind(thread_id.0)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn list_threads(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    parent_run_id: &str,
) -> Result<Vec<Thread>, StoreError> {
    let parent_run_id = match normalize_parent_run_id(parent_run_id) {
        Some(id) => id,
        None => return Ok(Vec::new()),
    };
    let sql = format!(
        "SELECT {THREAD_COLS}
         FROM maidan_threads t
         JOIN maidan_thread_lineage l ON l.thread_id = t.id
         JOIN maidan_channels c ON c.id = t.channel_id
         WHERE c.workspace_id = ?
           AND l.parent_run_id = ?
           AND t.tombstoned_at IS NULL
         ORDER BY t.created_at ASC, t.id"
    );
    let rows = sqlx::query(&sql)
        .bind(workspace_id.0)
        .bind(parent_run_id)
        .fetch_all(pool)
        .await?;
    rows.iter().map(row_to_thread).collect()
}

pub async fn occupancy(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    parent_run_id: &str,
) -> Result<RunOccupancy, StoreError> {
    let parent_run_id = match normalize_parent_run_id(parent_run_id) {
        Some(id) => id.to_string(),
        None => {
            return Ok(RunOccupancy {
                parent_run_id: parent_run_id.trim().to_string(),
                open: 0,
                queued: 0,
                claimed: 0,
                working: 0,
                blocked: 0,
            });
        }
    };
    let now = Utc::now().to_rfc3339();
    // The partition is `channel_occupancy`'s, built from the same
    // [`crate::claimability`] rules `claim_next` uses, so the two views of one
    // thread cannot disagree. Mute is not in this query (F7 stays orthogonal).
    let sql = crate::queue_counts::run_occupancy_select(&crate::queue_counts::RunSql {
        workspace: "?2",
        parent_run_id: "?3",
        thread: crate::claimability::ThreadSql {
            now: "?1",
            lapsed_worked_secs: crate::sqlite::threads::LAPSED_WORKED_SECS,
            requeued_after_result: crate::sqlite::threads::REQUEUED_AFTER_RESULT,
        },
    });
    let row = sqlx::query(&sql)
        .bind(&now)
        .bind(workspace_id.0)
        .bind(&parent_run_id)
        .fetch_one(pool)
        .await?;
    Ok(RunOccupancy {
        parent_run_id,
        open: row.get::<i64, _>("open_count"),
        queued: row.get::<i64, _>("queued_count"),
        claimed: row.get::<i64, _>("claimed_count"),
        working: row.get::<i64, _>("working_count"),
        blocked: row.get::<i64, _>("blocked_count"),
    })
}

fn row_to_lineage(row: &sqlx::sqlite::SqliteRow) -> Result<ThreadLineage, StoreError> {
    Ok(ThreadLineage {
        thread_id: ThreadId(row.get::<Uuid, _>("thread_id")),
        parent_run_id: row.get::<String, _>("parent_run_id"),
        set_at: row.get::<DateTime<Utc>, _>("set_at"),
    })
}

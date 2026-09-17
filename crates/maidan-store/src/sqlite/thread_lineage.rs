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

const THREAD_COLS: &str = "t.id, t.channel_id, t.parent_thread_id, t.title, t.state, \
     t.created_at, t.updated_at, t.tombstoned_at, t.assignee_id, \
     t.assignment_expires_at, t.claim_lease_id, t.work_started_at, t.owner_id";

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
    // Same predicates as `threads::channel_occupancy`, and they have to stay
    // the same: the two views answer the same question about the same threads,
    // so a divergence is a contradiction rather than a nuance. `blocked` counts
    // an explicit `maidan_thread_blocks` row as well as an unsatisfied
    // dependency, because `claim_next` skips both — reporting a block row as
    // `queued` advertises work that can never be claimed. Mute is not in this
    // query (F7 stays orthogonal).
    let row = sqlx::query(
        "SELECT
             COUNT(*) AS open_count,
             COALESCE(SUM(CASE WHEN t.assignee_id IS NOT NULL
                       AND (t.assignment_expires_at IS NULL OR t.assignment_expires_at >= ?)
                       AND t.work_started_at IS NULL
                     THEN 1 ELSE 0 END), 0) AS claimed_count,
             COALESCE(SUM(CASE WHEN t.assignee_id IS NOT NULL
                       AND (t.assignment_expires_at IS NULL OR t.assignment_expires_at >= ?)
                       AND t.work_started_at IS NOT NULL
                     THEN 1 ELSE 0 END), 0) AS working_count,
             COALESCE(SUM(CASE WHEN (t.assignee_id IS NULL OR (t.assignment_expires_at IS NOT NULL AND t.assignment_expires_at < ?))
                       AND NOT EXISTS (SELECT 1 FROM maidan_thread_blocks b WHERE b.thread_id = t.id)
                       AND NOT EXISTS (
                           SELECT 1 FROM maidan_thread_dependencies d
                           JOIN maidan_threads dep ON dep.id = d.depends_on_thread_id
                           WHERE d.thread_id = t.id AND dep.state NOT IN ('closed', 'archived'))
                     THEN 1 ELSE 0 END), 0) AS queued_count,
             COALESCE(SUM(CASE WHEN (t.assignee_id IS NULL OR (t.assignment_expires_at IS NOT NULL AND t.assignment_expires_at < ?))
                       AND (
                           EXISTS (SELECT 1 FROM maidan_thread_blocks b WHERE b.thread_id = t.id)
                           OR EXISTS (
                           SELECT 1 FROM maidan_thread_dependencies d
                           JOIN maidan_threads dep ON dep.id = d.depends_on_thread_id
                           WHERE d.thread_id = t.id AND dep.state NOT IN ('closed', 'archived'))
                       )
                     THEN 1 ELSE 0 END), 0) AS blocked_count
         FROM maidan_threads t
         JOIN maidan_thread_lineage l ON l.thread_id = t.id
         JOIN maidan_channels c ON c.id = t.channel_id
         WHERE c.workspace_id = ?
           AND l.parent_run_id = ?
           AND t.state NOT IN ('closed', 'archived')
           AND t.tombstoned_at IS NULL",
    )
    .bind(&now)
    .bind(&now)
    .bind(&now)
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

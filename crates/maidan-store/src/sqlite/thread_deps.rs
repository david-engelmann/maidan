use chrono::{DateTime, Utc};
use maidan_types::{Thread, ThreadDependency, ThreadId};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;

/// Add a task-dependency edge: `thread_id` depends on `depends_on` (Cluster 217).
/// Idempotent (`ON CONFLICT DO NOTHING`); a self-dependency is rejected. The FKs
/// require both threads to exist. A dependency that would close a cycle is
/// rejected (Cluster 221) — such a loop could never become ready.
pub async fn add(
    pool: &SqlitePool,
    thread_id: ThreadId,
    depends_on: ThreadId,
) -> Result<(), StoreError> {
    if thread_id == depends_on {
        return Err(StoreError::InvalidInput(
            "a thread cannot depend on itself".into(),
        ));
    }
    let mut tx = pool.begin().await?;
    // Cycle guard: reject if `depends_on` already (transitively) depends on
    // `thread_id`. Walking depends-on edges outward from `depends_on`, if we can
    // reach `thread_id` then adding `thread_id -> depends_on` closes a loop. The
    // check + insert share a transaction so a concurrent add can't interleave
    // between them.
    let cycle = sqlx::query(
        "WITH RECURSIVE reachable(id) AS (
             SELECT depends_on_thread_id FROM maidan_thread_dependencies WHERE thread_id = ?
             UNION
             SELECT d.depends_on_thread_id
             FROM maidan_thread_dependencies d
             JOIN reachable r ON d.thread_id = r.id
         )
         SELECT 1 AS hit FROM reachable WHERE id = ? LIMIT 1",
    )
    .bind(depends_on.0)
    .bind(thread_id.0)
    .fetch_optional(&mut *tx)
    .await?;
    if cycle.is_some() {
        return Err(StoreError::InvalidInput(
            "adding this dependency would create a cycle".into(),
        ));
    }
    sqlx::query(
        "INSERT INTO maidan_thread_dependencies (thread_id, depends_on_thread_id, created_at)
         VALUES (?, ?, ?)
         ON CONFLICT DO NOTHING",
    )
    .bind(thread_id.0)
    .bind(depends_on.0)
    .bind(Utc::now())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Remove a dependency edge; `true` when a row was deleted (Cluster 217).
pub async fn remove(
    pool: &SqlitePool,
    thread_id: ThreadId,
    depends_on: ThreadId,
) -> Result<bool, StoreError> {
    let res = sqlx::query(
        "DELETE FROM maidan_thread_dependencies
         WHERE thread_id = ? AND depends_on_thread_id = ?",
    )
    .bind(thread_id.0)
    .bind(depends_on.0)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// Edges `thread_id` depends on — what this task is blocked by (Cluster 217).
pub async fn list_dependencies(
    pool: &SqlitePool,
    thread_id: ThreadId,
) -> Result<Vec<ThreadDependency>, StoreError> {
    let rows = sqlx::query(
        "SELECT thread_id, depends_on_thread_id, created_at
         FROM maidan_thread_dependencies
         WHERE thread_id = ?
         ORDER BY created_at ASC",
    )
    .bind(thread_id.0)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_dep).collect()
}

/// Edges that depend on `thread_id` — what this task blocks (Cluster 217).
pub async fn list_dependents(
    pool: &SqlitePool,
    thread_id: ThreadId,
) -> Result<Vec<ThreadDependency>, StoreError> {
    let rows = sqlx::query(
        "SELECT thread_id, depends_on_thread_id, created_at
         FROM maidan_thread_dependencies
         WHERE depends_on_thread_id = ?
         ORDER BY created_at ASC",
    )
    .bind(thread_id.0)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_dep).collect()
}

/// Whether every dependency of `thread_id` is terminal (closed/archived) — i.e.
/// the task is ready (Cluster 217). A task with no dependencies is ready. A
/// hard-deleted dependency thread cascades its edge away, so it can't block; a
/// soft-tombstoned dependency keeps its last state and blocks unless that state
/// was terminal.
pub async fn dependencies_satisfied(
    pool: &SqlitePool,
    thread_id: ThreadId,
) -> Result<bool, StoreError> {
    let row = sqlx::query(
        "SELECT COUNT(*) AS pending
         FROM maidan_thread_dependencies d
         JOIN maidan_threads t ON t.id = d.depends_on_thread_id
         WHERE d.thread_id = ?
           AND t.state NOT IN ('closed', 'archived')",
    )
    .bind(thread_id.0)
    .fetch_one(pool)
    .await?;
    Ok(row.get::<i64, _>("pending") == 0)
}

/// Non-terminal dependents of `thread_id` whose dependencies are all terminal now
/// (Cluster 222). Called right after `thread_id` transitions to terminal: each row
/// is a task that just became ready. A dependent is included when it is itself
/// non-terminal AND has no non-terminal dependency remaining.
pub async fn newly_ready_dependents(
    pool: &SqlitePool,
    thread_id: ThreadId,
) -> Result<Vec<Thread>, StoreError> {
    let rows = sqlx::query(
        "SELECT t.* FROM maidan_threads t
         JOIN maidan_thread_dependencies d ON d.thread_id = t.id
         WHERE d.depends_on_thread_id = ?
           AND t.state NOT IN ('closed', 'archived')
           AND NOT EXISTS (
               SELECT 1 FROM maidan_thread_dependencies dd
               JOIN maidan_threads dep ON dep.id = dd.depends_on_thread_id
               WHERE dd.thread_id = t.id
                 AND dep.state NOT IN ('closed', 'archived')
           )
         ORDER BY t.created_at ASC",
    )
    .bind(thread_id.0)
    .fetch_all(pool)
    .await?;
    rows.iter().map(super::threads::row_to_thread).collect()
}

fn row_to_dep(row: &sqlx::sqlite::SqliteRow) -> Result<ThreadDependency, StoreError> {
    Ok(ThreadDependency {
        thread_id: ThreadId(row.get::<Uuid, _>("thread_id")),
        depends_on_thread_id: ThreadId(row.get::<Uuid, _>("depends_on_thread_id")),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
    })
}

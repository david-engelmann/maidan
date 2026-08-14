use chrono::{DateTime, Utc};
use maidan_types::{ThreadDependency, ThreadId};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;

/// Add a task-dependency edge: `thread_id` depends on `depends_on` (Cluster 217).
/// Idempotent (`ON CONFLICT DO NOTHING`); a self-dependency is rejected. The FKs
/// require both threads to exist.
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
    sqlx::query(
        "INSERT INTO maidan_thread_dependencies (thread_id, depends_on_thread_id, created_at)
         VALUES (?, ?, ?)
         ON CONFLICT DO NOTHING",
    )
    .bind(thread_id.0)
    .bind(depends_on.0)
    .bind(Utc::now())
    .execute(pool)
    .await?;
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

fn row_to_dep(row: &sqlx::sqlite::SqliteRow) -> Result<ThreadDependency, StoreError> {
    Ok(ThreadDependency {
        thread_id: ThreadId(row.get::<Uuid, _>("thread_id")),
        depends_on_thread_id: ThreadId(row.get::<Uuid, _>("depends_on_thread_id")),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
    })
}

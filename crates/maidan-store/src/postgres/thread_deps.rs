use chrono::{DateTime, Utc};
use maidan_types::{ThreadDependency, ThreadId};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::StoreError;

/// Add a task-dependency edge (Cluster 217) — see the SQLite twin.
pub async fn add(
    pool: &PgPool,
    thread_id: ThreadId,
    depends_on: ThreadId,
) -> Result<(), StoreError> {
    if thread_id == depends_on {
        return Err(StoreError::InvalidInput(
            "a thread cannot depend on itself".into(),
        ));
    }
    sqlx::query(
        "INSERT INTO maidan_thread_dependencies (thread_id, depends_on_thread_id)
         VALUES ($1, $2)
         ON CONFLICT DO NOTHING",
    )
    .bind(thread_id.0)
    .bind(depends_on.0)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn remove(
    pool: &PgPool,
    thread_id: ThreadId,
    depends_on: ThreadId,
) -> Result<bool, StoreError> {
    let res = sqlx::query(
        "DELETE FROM maidan_thread_dependencies
         WHERE thread_id = $1 AND depends_on_thread_id = $2",
    )
    .bind(thread_id.0)
    .bind(depends_on.0)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

pub async fn list_dependencies(
    pool: &PgPool,
    thread_id: ThreadId,
) -> Result<Vec<ThreadDependency>, StoreError> {
    let rows = sqlx::query(
        "SELECT thread_id, depends_on_thread_id, created_at
         FROM maidan_thread_dependencies
         WHERE thread_id = $1
         ORDER BY created_at ASC",
    )
    .bind(thread_id.0)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_dep).collect()
}

pub async fn list_dependents(
    pool: &PgPool,
    thread_id: ThreadId,
) -> Result<Vec<ThreadDependency>, StoreError> {
    let rows = sqlx::query(
        "SELECT thread_id, depends_on_thread_id, created_at
         FROM maidan_thread_dependencies
         WHERE depends_on_thread_id = $1
         ORDER BY created_at ASC",
    )
    .bind(thread_id.0)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_dep).collect()
}

pub async fn dependencies_satisfied(
    pool: &PgPool,
    thread_id: ThreadId,
) -> Result<bool, StoreError> {
    let row = sqlx::query(
        "SELECT COUNT(*) AS pending
         FROM maidan_thread_dependencies d
         JOIN maidan_threads t ON t.id = d.depends_on_thread_id
         WHERE d.thread_id = $1
           AND t.state NOT IN ('closed', 'archived')",
    )
    .bind(thread_id.0)
    .fetch_one(pool)
    .await?;
    Ok(row.get::<i64, _>("pending") == 0)
}

fn row_to_dep(row: &sqlx::postgres::PgRow) -> Result<ThreadDependency, StoreError> {
    Ok(ThreadDependency {
        thread_id: ThreadId(row.get::<Uuid, _>("thread_id")),
        depends_on_thread_id: ThreadId(row.get::<Uuid, _>("depends_on_thread_id")),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
    })
}

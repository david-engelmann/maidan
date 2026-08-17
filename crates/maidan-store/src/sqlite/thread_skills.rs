use chrono::{DateTime, Utc};
use maidan_types::{ThreadId, ThreadRequiredSkill};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;

/// Add a required skill to a task (Cluster 231). Idempotent; empty skill rejected.
pub async fn add(pool: &SqlitePool, thread_id: ThreadId, skill: &str) -> Result<(), StoreError> {
    if skill.trim().is_empty() {
        return Err(StoreError::InvalidInput("skill must not be empty".into()));
    }
    sqlx::query(
        "INSERT INTO maidan_thread_required_skills (thread_id, skill, created_at)
         VALUES (?, ?, ?)
         ON CONFLICT DO NOTHING",
    )
    .bind(thread_id.0)
    .bind(skill)
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

/// Remove a task's required skill; `true` when a row was deleted (Cluster 231).
pub async fn remove(
    pool: &SqlitePool,
    thread_id: ThreadId,
    skill: &str,
) -> Result<bool, StoreError> {
    let res =
        sqlx::query("DELETE FROM maidan_thread_required_skills WHERE thread_id = ? AND skill = ?")
            .bind(thread_id.0)
            .bind(skill)
            .execute(pool)
            .await?;
    Ok(res.rows_affected() > 0)
}

/// A task's required skills, ordered by skill (Cluster 231).
pub async fn list(
    pool: &SqlitePool,
    thread_id: ThreadId,
) -> Result<Vec<ThreadRequiredSkill>, StoreError> {
    let rows = sqlx::query(
        "SELECT thread_id, skill, created_at
         FROM maidan_thread_required_skills
         WHERE thread_id = ?
         ORDER BY skill ASC",
    )
    .bind(thread_id.0)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_required).collect())
}

fn row_to_required(row: &sqlx::sqlite::SqliteRow) -> ThreadRequiredSkill {
    ThreadRequiredSkill {
        thread_id: ThreadId(row.get::<Uuid, _>("thread_id")),
        skill: row.get("skill"),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
    }
}

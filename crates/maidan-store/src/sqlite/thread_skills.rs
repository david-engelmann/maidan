use chrono::{DateTime, Utc};
use maidan_types::{ThreadId, ThreadRequiredSkill};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;

/// Add a required skill to a task. Idempotent; empty skill rejected. A
/// required skill only steers dispatch, so a thread that is gone, closed or
/// archived refuses it rather than storing a rule nothing reads.
pub async fn add(pool: &SqlitePool, thread_id: ThreadId, skill: &str) -> Result<(), StoreError> {
    if skill.trim().is_empty() {
        return Err(StoreError::InvalidInput("skill must not be empty".into()));
    }
    let mut tx = super::tx::begin_write(pool).await?;
    let row = sqlx::query(
        "SELECT state, tombstoned_at IS NOT NULL AS gone FROM maidan_threads WHERE id = ?",
    )
    .bind(thread_id.0)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or(StoreError::NotFound)?;
    crate::thread_liveness::refuse_ended_column(
        thread_id,
        row.get::<&str, _>("state"),
        row.get("gone"),
        "a required skill",
    )?;
    sqlx::query(
        "INSERT INTO maidan_thread_required_skills (thread_id, skill, created_at)
         VALUES (?, ?, ?)
         ON CONFLICT DO NOTHING",
    )
    .bind(thread_id.0)
    .bind(skill)
    .bind(Utc::now().to_rfc3339())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Remove a task's required skill; `true` when a row was deleted.
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

/// A task's required skills, ordered by skill.
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

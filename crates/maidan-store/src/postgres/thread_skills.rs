use chrono::{DateTime, Utc};
use maidan_types::{ThreadId, ThreadRequiredSkill};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::StoreError;

/// Add a required skill to a task — see the SQLite twin. The thread row is
/// locked, so a close cannot land between the check and the insert.
pub async fn add(pool: &PgPool, thread_id: ThreadId, skill: &str) -> Result<(), StoreError> {
    if skill.trim().is_empty() {
        return Err(StoreError::InvalidInput("skill must not be empty".into()));
    }
    let mut tx = pool.begin().await?;
    let row = sqlx::query(
        "SELECT state, tombstoned_at IS NOT NULL AS gone FROM maidan_threads
         WHERE id = $1 FOR SHARE",
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
        "INSERT INTO maidan_thread_required_skills (thread_id, skill)
         VALUES ($1, $2)
         ON CONFLICT DO NOTHING",
    )
    .bind(thread_id.0)
    .bind(skill)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn remove(pool: &PgPool, thread_id: ThreadId, skill: &str) -> Result<bool, StoreError> {
    let res = sqlx::query(
        "DELETE FROM maidan_thread_required_skills WHERE thread_id = $1 AND skill = $2",
    )
    .bind(thread_id.0)
    .bind(skill)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

pub async fn list(
    pool: &PgPool,
    thread_id: ThreadId,
) -> Result<Vec<ThreadRequiredSkill>, StoreError> {
    let rows = sqlx::query(
        "SELECT thread_id, skill, created_at
         FROM maidan_thread_required_skills
         WHERE thread_id = $1
         ORDER BY skill ASC",
    )
    .bind(thread_id.0)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_required).collect())
}

fn row_to_required(row: &sqlx::postgres::PgRow) -> ThreadRequiredSkill {
    ThreadRequiredSkill {
        thread_id: ThreadId(row.get::<Uuid, _>("thread_id")),
        skill: row.get("skill"),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
    }
}

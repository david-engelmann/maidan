use chrono::{DateTime, Utc};
use maidan_types::{MemberId, MemberSkill};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;

/// Declare a skill for a member (Cluster 230). Idempotent; an empty skill is
/// rejected. The FK requires the member to exist.
pub async fn add(pool: &SqlitePool, member_id: MemberId, skill: &str) -> Result<(), StoreError> {
    if skill.trim().is_empty() {
        return Err(StoreError::InvalidInput("skill must not be empty".into()));
    }
    sqlx::query(
        "INSERT INTO maidan_member_skills (member_id, skill, created_at)
         VALUES (?, ?, ?)
         ON CONFLICT DO NOTHING",
    )
    .bind(member_id.0)
    .bind(skill)
    .bind(Utc::now().to_rfc3339())
    .execute(pool)
    .await?;
    Ok(())
}

/// Remove a member's skill; `true` when a row was deleted (Cluster 230).
pub async fn remove(
    pool: &SqlitePool,
    member_id: MemberId,
    skill: &str,
) -> Result<bool, StoreError> {
    let res = sqlx::query("DELETE FROM maidan_member_skills WHERE member_id = ? AND skill = ?")
        .bind(member_id.0)
        .bind(skill)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() > 0)
}

/// A member's declared skills, ordered by skill (Cluster 230).
pub async fn list(pool: &SqlitePool, member_id: MemberId) -> Result<Vec<MemberSkill>, StoreError> {
    let rows = sqlx::query(
        "SELECT member_id, skill, created_at
         FROM maidan_member_skills
         WHERE member_id = ?
         ORDER BY skill ASC",
    )
    .bind(member_id.0)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_skill).collect())
}

fn row_to_skill(row: &sqlx::sqlite::SqliteRow) -> MemberSkill {
    MemberSkill {
        member_id: MemberId(row.get::<Uuid, _>("member_id")),
        skill: row.get("skill"),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
    }
}

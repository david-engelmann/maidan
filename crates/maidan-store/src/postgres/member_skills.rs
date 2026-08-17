use chrono::{DateTime, Utc};
use maidan_types::{MemberId, MemberSkill};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::StoreError;

/// Declare a skill for a member (Cluster 230) — see the SQLite twin.
pub async fn add(pool: &PgPool, member_id: MemberId, skill: &str) -> Result<(), StoreError> {
    if skill.trim().is_empty() {
        return Err(StoreError::InvalidInput("skill must not be empty".into()));
    }
    sqlx::query(
        "INSERT INTO maidan_member_skills (member_id, skill)
         VALUES ($1, $2)
         ON CONFLICT DO NOTHING",
    )
    .bind(member_id.0)
    .bind(skill)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn remove(pool: &PgPool, member_id: MemberId, skill: &str) -> Result<bool, StoreError> {
    let res = sqlx::query("DELETE FROM maidan_member_skills WHERE member_id = $1 AND skill = $2")
        .bind(member_id.0)
        .bind(skill)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() > 0)
}

pub async fn list(pool: &PgPool, member_id: MemberId) -> Result<Vec<MemberSkill>, StoreError> {
    let rows = sqlx::query(
        "SELECT member_id, skill, created_at
         FROM maidan_member_skills
         WHERE member_id = $1
         ORDER BY skill ASC",
    )
    .bind(member_id.0)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_skill).collect())
}

fn row_to_skill(row: &sqlx::postgres::PgRow) -> MemberSkill {
    MemberSkill {
        member_id: MemberId(row.get::<Uuid, _>("member_id")),
        skill: row.get("skill"),
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
    }
}

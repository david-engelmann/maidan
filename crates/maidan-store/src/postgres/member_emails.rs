use chrono::{DateTime, Utc};
use maidan_types::{MemberEmail, MemberId};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::StoreError;

/// Set (upsert) a member's delivery email (Cluster 248) — see the SQLite twin.
pub async fn set(
    pool: &PgPool,
    member_id: MemberId,
    email: &str,
) -> Result<MemberEmail, StoreError> {
    let row = sqlx::query(
        "INSERT INTO maidan_member_emails (member_id, email)
         VALUES ($1, $2)
         ON CONFLICT (member_id) DO UPDATE SET email = excluded.email, updated_at = now()
         RETURNING member_id, email, updated_at",
    )
    .bind(member_id.0)
    .bind(email)
    .fetch_one(pool)
    .await?;
    Ok(row_to_email(&row))
}

pub async fn get(pool: &PgPool, member_id: MemberId) -> Result<Option<MemberEmail>, StoreError> {
    let row = sqlx::query(
        "SELECT member_id, email, updated_at FROM maidan_member_emails WHERE member_id = $1",
    )
    .bind(member_id.0)
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(row_to_email))
}

pub async fn delete(pool: &PgPool, member_id: MemberId) -> Result<bool, StoreError> {
    let res = sqlx::query("DELETE FROM maidan_member_emails WHERE member_id = $1")
        .bind(member_id.0)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() > 0)
}

fn row_to_email(row: &sqlx::postgres::PgRow) -> MemberEmail {
    MemberEmail {
        member_id: MemberId(row.get::<Uuid, _>("member_id")),
        email: row.get("email"),
        updated_at: row.get::<DateTime<Utc>, _>("updated_at"),
    }
}

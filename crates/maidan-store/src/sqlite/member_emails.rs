use chrono::{DateTime, Utc};
use maidan_types::{MemberEmail, MemberId};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;

/// Set (upsert) a member's delivery email (Cluster 248). A re-set overwrites.
pub async fn set(
    pool: &SqlitePool,
    member_id: MemberId,
    email: &str,
) -> Result<MemberEmail, StoreError> {
    let now = Utc::now().to_rfc3339();
    let row = sqlx::query(
        "INSERT INTO maidan_member_emails (member_id, email, updated_at)
         VALUES (?, ?, ?)
         ON CONFLICT (member_id) DO UPDATE SET email = excluded.email, updated_at = excluded.updated_at
         RETURNING member_id, email, updated_at",
    )
    .bind(member_id.0)
    .bind(email)
    .bind(&now)
    .fetch_one(pool)
    .await?;
    Ok(row_to_email(&row))
}

/// A member's delivery email, or `None` if unset (Cluster 248).
pub async fn get(
    pool: &SqlitePool,
    member_id: MemberId,
) -> Result<Option<MemberEmail>, StoreError> {
    let row = sqlx::query(
        "SELECT member_id, email, updated_at FROM maidan_member_emails WHERE member_id = ?",
    )
    .bind(member_id.0)
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(row_to_email))
}

/// Remove a member's delivery email; `true` when a row was deleted (Cluster 248).
pub async fn delete(pool: &SqlitePool, member_id: MemberId) -> Result<bool, StoreError> {
    let res = sqlx::query("DELETE FROM maidan_member_emails WHERE member_id = ?")
        .bind(member_id.0)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() > 0)
}

fn row_to_email(row: &sqlx::sqlite::SqliteRow) -> MemberEmail {
    MemberEmail {
        member_id: MemberId(row.get::<Uuid, _>("member_id")),
        email: row.get("email"),
        updated_at: row.get::<DateTime<Utc>, _>("updated_at"),
    }
}

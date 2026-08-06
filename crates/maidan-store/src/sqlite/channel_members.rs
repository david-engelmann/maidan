use chrono::{DateTime, Utc};
use maidan_types::{ChannelId, ChannelMember, ChannelMemberRole, MemberId};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;

fn row_to_member(row: &sqlx::sqlite::SqliteRow) -> Result<ChannelMember, StoreError> {
    let role_str: String = row.get("role");
    let role = ChannelMemberRole::parse(&role_str)
        .ok_or_else(|| StoreError::InvalidInput(format!("unknown channel role: {role_str}")))?;
    let created: String = row.get("created_at");
    let created_at = DateTime::parse_from_rfc3339(&created)
        .map(|t| t.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());
    Ok(ChannelMember {
        channel_id: ChannelId(row.get::<Uuid, _>("channel_id")),
        member_id: MemberId(row.get::<Uuid, _>("member_id")),
        role,
        created_at,
    })
}

pub async fn add(
    pool: &SqlitePool,
    channel_id: ChannelId,
    member_id: MemberId,
    role: ChannelMemberRole,
) -> Result<ChannelMember, StoreError> {
    let now = Utc::now().to_rfc3339();
    // Idempotent upsert; created_at is preserved on the update path.
    sqlx::query(
        "INSERT INTO maidan_channel_members (channel_id, member_id, role, created_at)
         VALUES (?, ?, ?, ?)
         ON CONFLICT (channel_id, member_id) DO UPDATE SET role = excluded.role",
    )
    .bind(channel_id.0)
    .bind(member_id.0)
    .bind(role.as_str())
    .bind(&now)
    .execute(pool)
    .await?;
    let row = sqlx::query(
        "SELECT channel_id, member_id, role, created_at
         FROM maidan_channel_members WHERE channel_id = ? AND member_id = ?",
    )
    .bind(channel_id.0)
    .bind(member_id.0)
    .fetch_one(pool)
    .await?;
    row_to_member(&row)
}

pub async fn remove(
    pool: &SqlitePool,
    channel_id: ChannelId,
    member_id: MemberId,
) -> Result<(), StoreError> {
    sqlx::query("DELETE FROM maidan_channel_members WHERE channel_id = ? AND member_id = ?")
        .bind(channel_id.0)
        .bind(member_id.0)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn list(
    pool: &SqlitePool,
    channel_id: ChannelId,
) -> Result<Vec<ChannelMember>, StoreError> {
    let rows = sqlx::query(
        "SELECT channel_id, member_id, role, created_at
         FROM maidan_channel_members WHERE channel_id = ?
         ORDER BY created_at, member_id",
    )
    .bind(channel_id.0)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_member).collect()
}

pub async fn is_member(
    pool: &SqlitePool,
    channel_id: ChannelId,
    member_id: MemberId,
) -> Result<bool, StoreError> {
    let found = sqlx::query_scalar::<_, i32>(
        "SELECT 1 FROM maidan_channel_members WHERE channel_id = ? AND member_id = ? LIMIT 1",
    )
    .bind(channel_id.0)
    .bind(member_id.0)
    .fetch_optional(pool)
    .await?;
    Ok(found.is_some())
}

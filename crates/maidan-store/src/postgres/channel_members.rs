use chrono::{DateTime, Utc};
use maidan_types::{ChannelId, ChannelMember, ChannelMemberRole, MemberId};
use sqlx::{PgPool, Row};

use crate::error::StoreError;

fn row_to_member(row: &sqlx::postgres::PgRow) -> Result<ChannelMember, StoreError> {
    let role_str: String = row.get("role");
    let role = ChannelMemberRole::parse(&role_str)
        .ok_or_else(|| StoreError::InvalidInput(format!("unknown channel role: {role_str}")))?;
    Ok(ChannelMember {
        channel_id: ChannelId(row.get("channel_id")),
        member_id: MemberId(row.get("member_id")),
        role,
        created_at: row.get::<DateTime<Utc>, _>("created_at"),
    })
}

pub async fn add(
    pool: &PgPool,
    channel_id: ChannelId,
    member_id: MemberId,
    role: ChannelMemberRole,
) -> Result<ChannelMember, StoreError> {
    let row = sqlx::query(
        "INSERT INTO maidan_channel_members (channel_id, member_id, role)
         VALUES ($1, $2, $3)
         ON CONFLICT (channel_id, member_id) DO UPDATE SET role = EXCLUDED.role
         RETURNING channel_id, member_id, role, created_at",
    )
    .bind(channel_id.0)
    .bind(member_id.0)
    .bind(role.as_str())
    .fetch_one(pool)
    .await?;
    row_to_member(&row)
}

pub async fn remove(
    pool: &PgPool,
    channel_id: ChannelId,
    member_id: MemberId,
) -> Result<(), StoreError> {
    sqlx::query("DELETE FROM maidan_channel_members WHERE channel_id = $1 AND member_id = $2")
        .bind(channel_id.0)
        .bind(member_id.0)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn list(pool: &PgPool, channel_id: ChannelId) -> Result<Vec<ChannelMember>, StoreError> {
    let rows = sqlx::query(
        "SELECT channel_id, member_id, role, created_at
         FROM maidan_channel_members WHERE channel_id = $1
         ORDER BY created_at, member_id",
    )
    .bind(channel_id.0)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_member).collect()
}

pub async fn is_member(
    pool: &PgPool,
    channel_id: ChannelId,
    member_id: MemberId,
) -> Result<bool, StoreError> {
    let found = sqlx::query_scalar::<_, i32>(
        "SELECT 1 FROM maidan_channel_members WHERE channel_id = $1 AND member_id = $2 LIMIT 1",
    )
    .bind(channel_id.0)
    .bind(member_id.0)
    .fetch_optional(pool)
    .await?;
    Ok(found.is_some())
}

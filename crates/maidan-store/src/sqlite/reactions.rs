use chrono::{DateTime, Utc};
use maidan_types::{MemberId, MessageId, NewReaction, Reaction};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;

const MAX_EMOJI_LEN: usize = 64;

pub fn normalize_emoji(emoji: &str) -> Result<String, StoreError> {
    let emoji = emoji.trim();
    if emoji.is_empty() {
        return Err(StoreError::InvalidInput("emoji must not be empty".into()));
    }
    if emoji.len() > MAX_EMOJI_LEN {
        return Err(StoreError::InvalidInput("emoji too long".into()));
    }
    Ok(emoji.to_string())
}

pub async fn add(pool: &SqlitePool, new: NewReaction) -> Result<(), StoreError> {
    let emoji = normalize_emoji(&new.emoji)?;
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO maidan_reactions (message_id, member_id, emoji, created_at)
         VALUES (?, ?, ?, ?)
         ON CONFLICT DO NOTHING",
    )
    .bind(new.message_id.0)
    .bind(new.member_id.0)
    .bind(&emoji)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn remove(
    pool: &SqlitePool,
    message_id: MessageId,
    member_id: MemberId,
    emoji: &str,
) -> Result<bool, StoreError> {
    let emoji = normalize_emoji(emoji)?;
    let result = sqlx::query(
        "DELETE FROM maidan_reactions
         WHERE message_id = ? AND member_id = ? AND emoji = ?",
    )
    .bind(message_id.0)
    .bind(member_id.0)
    .bind(&emoji)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn list(pool: &SqlitePool, message_id: MessageId) -> Result<Vec<Reaction>, StoreError> {
    let rows = sqlx::query(
        "SELECT message_id, member_id, emoji, created_at
         FROM maidan_reactions
         WHERE message_id = ?
         ORDER BY created_at ASC",
    )
    .bind(message_id.0)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|row| Reaction {
            message_id: MessageId(row.get::<Uuid, _>("message_id")),
            member_id: MemberId(row.get::<Uuid, _>("member_id")),
            emoji: row.get("emoji"),
            created_at: row.get::<DateTime<Utc>, _>("created_at"),
        })
        .collect())
}

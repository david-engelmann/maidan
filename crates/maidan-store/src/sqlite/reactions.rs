use chrono::{DateTime, Utc};
use maidan_types::{Event, MemberId, MessageId, NewReaction, Reaction, StoredEvent};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;
use crate::sqlite::events;

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

/// Add a reaction and append its `ReactionAdded` event in one transaction
/// (Cluster 206).
pub async fn add_with_event(
    pool: &SqlitePool,
    new: NewReaction,
) -> Result<StoredEvent, StoreError> {
    let emoji = normalize_emoji(&new.emoji)?;
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO maidan_reactions (message_id, member_id, emoji, created_at)
         VALUES (?, ?, ?, ?)
         ON CONFLICT DO NOTHING",
    )
    .bind(new.message_id.0)
    .bind(new.member_id.0)
    .bind(&emoji)
    .bind(Utc::now())
    .execute(&mut *tx)
    .await?;
    let (workspace_id, _channel_id, thread_id) =
        events::message_scope_in_tx(&mut tx, new.message_id).await?;
    let event = Event::ReactionAdded {
        occurred_at: Utc::now(),
        workspace_id,
        thread_id,
        message_id: new.message_id,
        member_id: new.member_id,
        emoji,
    };
    let stored = events::append_in_tx(&mut tx, &event).await?;
    tx.commit().await?;
    Ok(stored)
}

/// Remove a reaction; when a row was actually removed, append its
/// `ReactionRemoved` event in the SAME transaction (Cluster 206). Returns
/// `(removed, event)` — no event when nothing was removed (idempotent no-op).
pub async fn remove_with_event(
    pool: &SqlitePool,
    message_id: MessageId,
    member_id: MemberId,
    emoji: &str,
) -> Result<(bool, Option<StoredEvent>), StoreError> {
    let emoji = normalize_emoji(emoji)?;
    let mut tx = pool.begin().await?;
    let result = sqlx::query(
        "DELETE FROM maidan_reactions
         WHERE message_id = ? AND member_id = ? AND emoji = ?",
    )
    .bind(message_id.0)
    .bind(member_id.0)
    .bind(&emoji)
    .execute(&mut *tx)
    .await?;
    if result.rows_affected() == 0 {
        tx.commit().await?;
        return Ok((false, None));
    }
    let (workspace_id, _channel_id, thread_id) =
        events::message_scope_in_tx(&mut tx, message_id).await?;
    let event = Event::ReactionRemoved {
        occurred_at: Utc::now(),
        workspace_id,
        thread_id,
        message_id,
        member_id,
        emoji,
    };
    let stored = events::append_in_tx(&mut tx, &event).await?;
    tx.commit().await?;
    Ok((true, Some(stored)))
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

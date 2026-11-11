use chrono::{DateTime, Utc};
use maidan_types::{Event, MemberId, Mention, MessageId, StoredEvent, DM_CHANNEL_NAME};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;
use crate::sqlite::{events, members};
use crate::thread_access::mention_visible_to;

/// Record a mention without an event. The `@handle` router does, and
/// publishes `MentionRecorded` itself. It checks the member against the
/// message's workspace, as [`record_with_event`] does.
pub async fn record(
    pool: &SqlitePool,
    message_id: MessageId,
    member_id: MemberId,
) -> Result<(), StoreError> {
    let mut tx = super::tx::begin_write(pool).await?;
    let (workspace_id, _channel_id, _thread_id) =
        events::message_scope_in_tx(&mut tx, message_id).await?;
    members::ensure_in_workspace(&mut tx, workspace_id, member_id).await?;
    sqlx::query(
        "INSERT INTO maidan_mentions (message_id, member_id, created_at)
         VALUES (?, ?, ?)
         ON CONFLICT DO NOTHING",
    )
    .bind(message_id.0)
    .bind(member_id.0)
    .bind(Utc::now())
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Record a mention and append its `MentionRecorded` event in one transaction.
/// `member_id` is the mentioned party.
pub async fn record_with_event(
    pool: &SqlitePool,
    message_id: MessageId,
    member_id: MemberId,
) -> Result<StoredEvent, StoreError> {
    let mut tx = super::tx::begin_write(pool).await?;
    let (workspace_id, _channel_id, thread_id) =
        events::message_scope_in_tx(&mut tx, message_id).await?;
    members::ensure_in_workspace(&mut tx, workspace_id, member_id).await?;
    sqlx::query(
        "INSERT INTO maidan_mentions (message_id, member_id, created_at)
         VALUES (?, ?, ?)
         ON CONFLICT DO NOTHING",
    )
    .bind(message_id.0)
    .bind(member_id.0)
    .bind(Utc::now())
    .execute(&mut *tx)
    .await?;
    let event = Event::MentionRecorded {
        occurred_at: Utc::now(),
        workspace_id,
        thread_id,
        message_id,
        member_id,
    };
    let stored = events::append_in_tx(&mut tx, &event).await?;
    tx.commit().await?;
    Ok(stored)
}

pub async fn list_for_member(
    pool: &SqlitePool,
    member_id: MemberId,
    limit: i64,
) -> Result<Vec<Mention>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT m.message_id, m.member_id, m.created_at
         FROM maidan_mentions m
         WHERE m.member_id = ?1 AND {}
         ORDER BY m.created_at DESC
         LIMIT ?2",
        mention_visible_to("m", "?1", "?3")
    ))
    .bind(member_id.0)
    .bind(limit)
    .bind(DM_CHANNEL_NAME)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|row| Mention {
            message_id: MessageId(row.get::<Uuid, _>("message_id")),
            member_id: MemberId(row.get::<Uuid, _>("member_id")),
            created_at: row.get::<DateTime<Utc>, _>("created_at"),
        })
        .collect())
}

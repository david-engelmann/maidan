use chrono::{DateTime, Utc};
use maidan_types::{Event, MemberId, MessageId, NewPin, Pin, StoredEvent, ThreadId};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::StoreError;
use crate::postgres::events;

pub async fn pin(pool: &PgPool, new: NewPin) -> Result<(), StoreError> {
    let message = super::messages::get(pool, new.message_id).await?;
    if message.thread_id != new.thread_id {
        return Err(StoreError::InvalidInput(
            "message does not belong to thread".into(),
        ));
    }
    sqlx::query(
        "INSERT INTO maidan_pins (thread_id, message_id, member_id)
         VALUES ($1, $2, $3)
         ON CONFLICT DO NOTHING",
    )
    .bind(new.thread_id.0)
    .bind(new.message_id.0)
    .bind(new.member_id.0)
    .execute(pool)
    .await?;
    Ok(())
}

/// Pin a message and append its `MessagePinned` event in one transaction
/// (Cluster 207).
pub async fn pin_with_event(pool: &PgPool, new: NewPin) -> Result<StoredEvent, StoreError> {
    let message = super::messages::get(pool, new.message_id).await?;
    if message.thread_id != new.thread_id {
        return Err(StoreError::InvalidInput(
            "message does not belong to thread".into(),
        ));
    }
    let mut tx = pool.begin().await?;
    sqlx::query(
        "INSERT INTO maidan_pins (thread_id, message_id, member_id)
         VALUES ($1, $2, $3)
         ON CONFLICT DO NOTHING",
    )
    .bind(new.thread_id.0)
    .bind(new.message_id.0)
    .bind(new.member_id.0)
    .execute(&mut *tx)
    .await?;
    let (workspace_id, channel_id, thread_id) =
        events::message_scope_in_tx(&mut tx, new.message_id).await?;
    let event = Event::MessagePinned {
        occurred_at: Utc::now(),
        workspace_id,
        channel_id,
        thread_id,
        message_id: new.message_id,
        member_id: new.member_id,
    };
    let stored = events::append_in_tx(&mut tx, &event).await?;
    tx.commit().await?;
    Ok(stored)
}

pub async fn unpin(
    pool: &PgPool,
    thread_id: ThreadId,
    message_id: MessageId,
) -> Result<bool, StoreError> {
    let result = sqlx::query("DELETE FROM maidan_pins WHERE thread_id = $1 AND message_id = $2")
        .bind(thread_id.0)
        .bind(message_id.0)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

/// Unpin a message; append its `MessageUnpinned` event in the SAME transaction
/// when a row was removed (Cluster 207). `member_id` is the actor for the event.
pub async fn unpin_with_event(
    pool: &PgPool,
    thread_id: ThreadId,
    message_id: MessageId,
    member_id: MemberId,
) -> Result<(bool, Option<StoredEvent>), StoreError> {
    let mut tx = pool.begin().await?;
    let result = sqlx::query("DELETE FROM maidan_pins WHERE thread_id = $1 AND message_id = $2")
        .bind(thread_id.0)
        .bind(message_id.0)
        .execute(&mut *tx)
        .await?;
    if result.rows_affected() == 0 {
        tx.commit().await?;
        return Ok((false, None));
    }
    let (workspace_id, channel_id, resolved_thread) =
        events::message_scope_in_tx(&mut tx, message_id).await?;
    let event = Event::MessageUnpinned {
        occurred_at: Utc::now(),
        workspace_id,
        channel_id,
        thread_id: resolved_thread,
        message_id,
        member_id,
    };
    let stored = events::append_in_tx(&mut tx, &event).await?;
    tx.commit().await?;
    Ok((true, Some(stored)))
}

pub async fn list_for_thread(pool: &PgPool, thread_id: ThreadId) -> Result<Vec<Pin>, StoreError> {
    let rows = sqlx::query(
        "SELECT thread_id, message_id, member_id, created_at
         FROM maidan_pins
         WHERE thread_id = $1
         ORDER BY created_at ASC",
    )
    .bind(thread_id.0)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|row| Pin {
            thread_id: ThreadId(row.get::<Uuid, _>("thread_id")),
            message_id: MessageId(row.get::<Uuid, _>("message_id")),
            member_id: MemberId(row.get::<Uuid, _>("member_id")),
            created_at: row.get::<DateTime<Utc>, _>("created_at"),
        })
        .collect())
}

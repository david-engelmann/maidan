use chrono::{DateTime, Utc};
use maidan_types::{MemberId, MessageId, NewPin, Pin, ThreadId};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;

pub async fn pin(pool: &SqlitePool, new: NewPin) -> Result<(), StoreError> {
    let message = super::messages::get(pool, new.message_id).await?;
    if message.thread_id != new.thread_id {
        return Err(StoreError::InvalidInput(
            "message does not belong to thread".into(),
        ));
    }
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO maidan_pins (thread_id, message_id, member_id, created_at)
         VALUES (?, ?, ?, ?)
         ON CONFLICT DO NOTHING",
    )
    .bind(new.thread_id.0)
    .bind(new.message_id.0)
    .bind(new.member_id.0)
    .bind(now)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn unpin(
    pool: &SqlitePool,
    thread_id: ThreadId,
    message_id: MessageId,
) -> Result<bool, StoreError> {
    let result = sqlx::query("DELETE FROM maidan_pins WHERE thread_id = ? AND message_id = ?")
        .bind(thread_id.0)
        .bind(message_id.0)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn list_for_thread(
    pool: &SqlitePool,
    thread_id: ThreadId,
) -> Result<Vec<Pin>, StoreError> {
    let rows = sqlx::query(
        "SELECT thread_id, message_id, member_id, created_at
         FROM maidan_pins
         WHERE thread_id = ?
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

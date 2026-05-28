use chrono::{DateTime, Utc};
use maidan_types::{MemberId, Message, MessageId, NewMessage, ThreadId};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;

pub async fn create(pool: &SqlitePool, new: NewMessage) -> Result<Message, StoreError> {
    let id = Uuid::new_v4();
    let now = Utc::now();
    let metadata_text = serde_json::to_string(&new.metadata)?;
    let row = sqlx::query(
        "INSERT INTO maidan_messages (id, thread_id, author_id, body, metadata, posted_at)
         VALUES (?, ?, ?, ?, ?, ?)
         RETURNING id, thread_id, author_id, body, metadata, posted_at, edited_at, tombstoned_at",
    )
    .bind(id)
    .bind(new.thread_id.0)
    .bind(new.author_id.0)
    .bind(&new.body)
    .bind(&metadata_text)
    .bind(now)
    .fetch_one(pool)
    .await?;
    row_to_message(&row)
}

pub async fn get(pool: &SqlitePool, id: MessageId) -> Result<Message, StoreError> {
    let row = sqlx::query(
        "SELECT id, thread_id, author_id, body, metadata, posted_at, edited_at, tombstoned_at
         FROM maidan_messages WHERE id = ?",
    )
    .bind(id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_message(&row)
}

pub async fn list(
    pool: &SqlitePool,
    thread_id: ThreadId,
    limit: i64,
) -> Result<Vec<Message>, StoreError> {
    let rows = sqlx::query(
        "SELECT id, thread_id, author_id, body, metadata, posted_at, edited_at, tombstoned_at
         FROM maidan_messages
         WHERE thread_id = ? AND tombstoned_at IS NULL
         ORDER BY posted_at ASC
         LIMIT ?",
    )
    .bind(thread_id.0)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_message).collect()
}

pub async fn purge(pool: &SqlitePool, id: MessageId) -> Result<(), StoreError> {
    let res = sqlx::query("DELETE FROM maidan_messages WHERE id = ? AND tombstoned_at IS NOT NULL")
        .bind(id.0)
        .execute(pool)
        .await?;
    if res.rows_affected() == 0 {
        return Err(StoreError::NotFound);
    }
    Ok(())
}

pub async fn tombstone(pool: &SqlitePool, id: MessageId) -> Result<(), StoreError> {
    let now = Utc::now();
    let res = sqlx::query(
        "UPDATE maidan_messages SET tombstoned_at = ?, body = '' WHERE id = ? AND tombstoned_at IS NULL",
    )
    .bind(now)
    .bind(id.0)
    .execute(pool)
    .await?;
    if res.rows_affected() == 0 {
        return Err(StoreError::NotFound);
    }
    Ok(())
}

fn row_to_message(row: &sqlx::sqlite::SqliteRow) -> Result<Message, StoreError> {
    let metadata_text: String = row.get("metadata");
    let metadata = serde_json::from_str(&metadata_text)?;
    Ok(Message {
        id: MessageId(row.get::<Uuid, _>("id")),
        thread_id: ThreadId(row.get::<Uuid, _>("thread_id")),
        author_id: MemberId(row.get::<Uuid, _>("author_id")),
        body: row.get("body"),
        metadata,
        posted_at: row.get::<DateTime<Utc>, _>("posted_at"),
        edited_at: row.get::<Option<DateTime<Utc>>, _>("edited_at"),
        tombstoned_at: row.get::<Option<DateTime<Utc>>, _>("tombstoned_at"),
    })
}

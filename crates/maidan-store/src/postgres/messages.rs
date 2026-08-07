use chrono::{DateTime, Utc};
use maidan_types::{EditMessage, MemberId, Message, MessageId, NewMessage, ThreadId};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::StoreError;

pub async fn create(pool: &PgPool, new: NewMessage) -> Result<Message, StoreError> {
    let id = Uuid::new_v4();
    let content = new.content.as_ref().map(serde_json::to_value).transpose()?;
    let row = sqlx::query(
        "INSERT INTO maidan_messages (id, thread_id, author_id, body, metadata, content)
         VALUES ($1, $2, $3, $4, $5, $6)
         RETURNING id, thread_id, author_id, body, metadata, content, posted_at, edited_at, tombstoned_at",
    )
    .bind(id)
    .bind(new.thread_id.0)
    .bind(new.author_id.0)
    .bind(&new.body)
    .bind(&new.metadata)
    .bind(content)
    .fetch_one(pool)
    .await?;
    Ok(row_to_message(&row))
}

pub async fn get(pool: &PgPool, id: MessageId) -> Result<Message, StoreError> {
    let row = sqlx::query(
        "SELECT id, thread_id, author_id, body, metadata, content, posted_at, edited_at, tombstoned_at
         FROM maidan_messages WHERE id = $1",
    )
    .bind(id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    Ok(row_to_message(&row))
}

pub async fn list(
    pool: &PgPool,
    thread_id: ThreadId,
    limit: i64,
) -> Result<Vec<Message>, StoreError> {
    let rows = sqlx::query(
        "SELECT id, thread_id, author_id, body, metadata, content, posted_at, edited_at, tombstoned_at
         FROM maidan_messages
         WHERE thread_id = $1 AND tombstoned_at IS NULL
         ORDER BY posted_at ASC, id ASC
         LIMIT $2",
    )
    .bind(thread_id.0)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_message).collect())
}

pub async fn list_after(
    pool: &PgPool,
    thread_id: ThreadId,
    after: Option<MessageId>,
    limit: i64,
) -> Result<Vec<Message>, StoreError> {
    let rows = match after {
        None => {
            sqlx::query(
                "SELECT id, thread_id, author_id, body, metadata, content, posted_at, edited_at, tombstoned_at
                 FROM maidan_messages
                 WHERE thread_id = $1 AND tombstoned_at IS NULL
                 ORDER BY posted_at ASC, id ASC
                 LIMIT $2",
            )
            .bind(thread_id.0)
            .bind(limit)
            .fetch_all(pool)
            .await?
        }
        Some(after_id) => {
            sqlx::query(
                "SELECT m.id, m.thread_id, m.author_id, m.body, m.metadata, m.content, m.posted_at, m.edited_at, m.tombstoned_at
                 FROM maidan_messages m
                 JOIN maidan_messages anchor ON anchor.id = $1
                 WHERE m.thread_id = $2 AND m.tombstoned_at IS NULL
                   AND (m.posted_at > anchor.posted_at
                        OR (m.posted_at = anchor.posted_at AND m.id > anchor.id))
                 ORDER BY m.posted_at ASC, m.id ASC
                 LIMIT $3",
            )
            .bind(after_id.0)
            .bind(thread_id.0)
            .bind(limit)
            .fetch_all(pool)
            .await?
        }
    };
    Ok(rows.iter().map(row_to_message).collect())
}

pub async fn purge(pool: &PgPool, id: MessageId) -> Result<(), StoreError> {
    let res =
        sqlx::query("DELETE FROM maidan_messages WHERE id = $1 AND tombstoned_at IS NOT NULL")
            .bind(id.0)
            .execute(pool)
            .await?;
    if res.rows_affected() == 0 {
        return Err(StoreError::NotFound);
    }
    Ok(())
}

pub async fn edit(
    pool: &PgPool,
    id: MessageId,
    editor_id: MemberId,
    edit: EditMessage,
) -> Result<Message, StoreError> {
    let existing = get(pool, id).await?;
    if existing.body != edit.body {
        super::message_edits::append(pool, id, editor_id, &existing.body, &edit.body, Utc::now())
            .await?;
    }
    let metadata = serde_json::to_value(&edit.metadata)?;
    let content = edit
        .content
        .as_ref()
        .map(serde_json::to_value)
        .transpose()?;
    let row = sqlx::query(
        "UPDATE maidan_messages SET body = $2, metadata = $3, content = $4, edited_at = NOW()
         WHERE id = $1 AND tombstoned_at IS NULL
         RETURNING id, thread_id, author_id, body, metadata, content, posted_at, edited_at, tombstoned_at",
    )
    .bind(id.0)
    .bind(&edit.body)
    .bind(metadata)
    .bind(content)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    Ok(row_to_message(&row))
}

pub async fn tombstone(pool: &PgPool, id: MessageId) -> Result<(), StoreError> {
    let res = sqlx::query(
        "UPDATE maidan_messages SET tombstoned_at = NOW(), body = '', content = NULL WHERE id = $1 AND tombstoned_at IS NULL",
    )
    .bind(id.0)
    .execute(pool)
    .await?;
    if res.rows_affected() == 0 {
        return Err(StoreError::NotFound);
    }
    Ok(())
}

fn row_to_message(row: &sqlx::postgres::PgRow) -> Message {
    Message {
        id: MessageId(row.get::<Uuid, _>("id")),
        thread_id: ThreadId(row.get::<Uuid, _>("thread_id")),
        author_id: MemberId(row.get::<Uuid, _>("author_id")),
        body: row.get("body"),
        metadata: row.get::<serde_json::Value, _>("metadata"),
        content: row
            .get::<Option<serde_json::Value>, _>("content")
            .and_then(|v| serde_json::from_value(v).ok()),
        posted_at: row.get::<DateTime<Utc>, _>("posted_at"),
        edited_at: row.get::<Option<DateTime<Utc>>, _>("edited_at"),
        tombstoned_at: row.get::<Option<DateTime<Utc>>, _>("tombstoned_at"),
    }
}

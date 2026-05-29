use chrono::{DateTime, Utc};
use maidan_types::{MemberId, MessageEdit, MessageId};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;

pub async fn append(
    pool: &SqlitePool,
    message_id: MessageId,
    editor_id: MemberId,
    body_before: &str,
    body_after: &str,
    edited_at: DateTime<Utc>,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO maidan_message_edits (message_id, editor_id, body_before, body_after, edited_at)
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(message_id.0)
    .bind(editor_id.0)
    .bind(body_before)
    .bind(body_after)
    .bind(edited_at)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list(
    pool: &SqlitePool,
    message_id: MessageId,
    limit: i64,
) -> Result<Vec<MessageEdit>, StoreError> {
    let rows = sqlx::query(
        "SELECT id, message_id, editor_id, body_before, body_after, edited_at
         FROM maidan_message_edits
         WHERE message_id = ?
         ORDER BY edited_at ASC, id ASC
         LIMIT ?",
    )
    .bind(message_id.0)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_edit).collect()
}

fn row_to_edit(row: &sqlx::sqlite::SqliteRow) -> Result<MessageEdit, StoreError> {
    Ok(MessageEdit {
        id: row.get("id"),
        message_id: MessageId(row.get::<Uuid, _>("message_id")),
        editor_id: MemberId(row.get::<Uuid, _>("editor_id")),
        body_before: row.get("body_before"),
        body_after: row.get("body_after"),
        edited_at: row.get::<DateTime<Utc>, _>("edited_at"),
    })
}

use chrono::{DateTime, Utc};
use maidan_types::{MemberId, MessageEdit, MessageId};
use sqlx::{PgPool, Row};

use crate::error::StoreError;

pub async fn append(
    pool: &PgPool,
    message_id: MessageId,
    editor_id: MemberId,
    body_before: &str,
    body_after: &str,
    edited_at: DateTime<Utc>,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO maidan_message_edits (message_id, editor_id, body_before, body_after, edited_at)
         VALUES ($1, $2, $3, $4, $5)",
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
    pool: &PgPool,
    message_id: MessageId,
    limit: i64,
) -> Result<Vec<MessageEdit>, StoreError> {
    let rows = sqlx::query(
        "SELECT id, message_id, editor_id, body_before, body_after, edited_at
         FROM maidan_message_edits
         WHERE message_id = $1
         ORDER BY edited_at ASC, id ASC
         LIMIT $2",
    )
    .bind(message_id.0)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_edit).collect())
}

fn row_to_edit(row: &sqlx::postgres::PgRow) -> MessageEdit {
    MessageEdit {
        id: row.get("id"),
        message_id: MessageId(row.get("message_id")),
        editor_id: MemberId(row.get("editor_id")),
        body_before: row.get("body_before"),
        body_after: row.get("body_after"),
        edited_at: row.get("edited_at"),
    }
}

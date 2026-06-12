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

/// SQLite has no array binding; expand `IN (?, …)` and chunk well under the
/// variable limit (one slot is reserved for `limit_per`).
const SQLITE_IN_CHUNK: usize = 400;

pub async fn list_for_messages(
    pool: &SqlitePool,
    message_ids: &[MessageId],
    limit_per: i64,
) -> Result<Vec<MessageEdit>, StoreError> {
    let mut out = Vec::new();
    for chunk in message_ids.chunks(SQLITE_IN_CHUNK) {
        let placeholders = vec!["?"; chunk.len()].join(", ");
        // Window the per-message edits so each message yields at most limit_per
        // (SQLite has window functions since 3.25).
        let sql = format!(
            "SELECT id, message_id, editor_id, body_before, body_after, edited_at
             FROM (
                 SELECT id, message_id, editor_id, body_before, body_after, edited_at,
                        ROW_NUMBER() OVER (
                            PARTITION BY message_id ORDER BY edited_at ASC, id ASC
                        ) AS rn
                 FROM maidan_message_edits
                 WHERE message_id IN ({placeholders})
             ) windowed
             WHERE rn <= ?
             ORDER BY message_id, edited_at ASC, id ASC"
        );
        let mut q = sqlx::query(&sql);
        for id in chunk {
            q = q.bind(id.0);
        }
        q = q.bind(limit_per);
        let rows = q.fetch_all(pool).await?;
        for row in &rows {
            out.push(row_to_edit(row)?);
        }
    }
    Ok(out)
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

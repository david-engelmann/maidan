use chrono::{DateTime, Utc};
use maidan_types::{
    MemberId, Message, MessageId, NewMessage, ThreadId, WorkspaceId, WorkspacePurgeResult,
};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::StoreError;

pub async fn create(pool: &PgPool, new: NewMessage) -> Result<Message, StoreError> {
    let id = Uuid::new_v4();
    let row = sqlx::query(
        "INSERT INTO maidan_messages (id, thread_id, author_id, body, metadata)
         VALUES ($1, $2, $3, $4, $5)
         RETURNING id, thread_id, author_id, body, metadata, posted_at, edited_at, tombstoned_at",
    )
    .bind(id)
    .bind(new.thread_id.0)
    .bind(new.author_id.0)
    .bind(&new.body)
    .bind(&new.metadata)
    .fetch_one(pool)
    .await?;
    Ok(row_to_message(&row))
}

pub async fn get(pool: &PgPool, id: MessageId) -> Result<Message, StoreError> {
    let row = sqlx::query(
        "SELECT id, thread_id, author_id, body, metadata, posted_at, edited_at, tombstoned_at
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
        "SELECT id, thread_id, author_id, body, metadata, posted_at, edited_at, tombstoned_at
         FROM maidan_messages
         WHERE thread_id = $1 AND tombstoned_at IS NULL
         ORDER BY posted_at ASC
         LIMIT $2",
    )
    .bind(thread_id.0)
    .bind(limit)
    .fetch_all(pool)
    .await?;
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

pub async fn purge_workspace(
    pool: &PgPool,
    workspace_id: WorkspaceId,
) -> Result<WorkspacePurgeResult, StoreError> {
    let tombstone = sqlx::query(
        "UPDATE maidan_messages SET tombstoned_at = NOW(), body = ''
         WHERE tombstoned_at IS NULL
           AND thread_id IN (
             SELECT t.id FROM maidan_threads t
             INNER JOIN maidan_channels c ON t.channel_id = c.id
             WHERE c.workspace_id = $1
           )",
    )
    .bind(workspace_id.0)
    .execute(pool)
    .await?;
    let purge = sqlx::query(
        "DELETE FROM maidan_messages
         WHERE tombstoned_at IS NOT NULL
           AND thread_id IN (
             SELECT t.id FROM maidan_threads t
             INNER JOIN maidan_channels c ON t.channel_id = c.id
             WHERE c.workspace_id = $1
           )",
    )
    .bind(workspace_id.0)
    .execute(pool)
    .await?;
    let occurred_at = Utc::now();
    Ok(WorkspacePurgeResult {
        workspace_id,
        messages_tombstoned: tombstone.rows_affected(),
        messages_purged: purge.rows_affected(),
        occurred_at,
    })
}

pub async fn tombstone(pool: &PgPool, id: MessageId) -> Result<(), StoreError> {
    let res = sqlx::query(
        "UPDATE maidan_messages SET tombstoned_at = NOW(), body = '' WHERE id = $1 AND tombstoned_at IS NULL",
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
        posted_at: row.get::<DateTime<Utc>, _>("posted_at"),
        edited_at: row.get::<Option<DateTime<Utc>>, _>("edited_at"),
        tombstoned_at: row.get::<Option<DateTime<Utc>>, _>("tombstoned_at"),
    }
}

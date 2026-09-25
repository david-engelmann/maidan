use chrono::{DateTime, Utc};
use maidan_types::{
    ChannelId, ContentBlock, DmConversationId, EditMessage, Event, MemberId, Message, MessageId,
    NewMessage, SpawnAxis, SpawnDenial, StoredEvent, ThreadId, WorkspaceId,
};
use sqlx::{Row, SqliteConnection, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;
use crate::sqlite::events;

/// Tool-use blocks in a new message's content.
fn new_tool_uses(content: &Option<Vec<ContentBlock>>) -> i64 {
    content.as_deref().map_or(0, |blocks| {
        blocks
            .iter()
            .filter(|b| matches!(b, ContentBlock::ToolUse { .. }))
            .count() as i64
    })
}

/// Enforce the workspace's `max_tools` axis on a post — refuse once the
/// thread's recorded tool calls + this post's would exceed the cap, as a typed
/// `SpawnRejected`. See the Postgres twin.
async fn enforce_tool_budget(
    pool: &SqlitePool,
    thread_id: ThreadId,
    adding: i64,
) -> Result<(), StoreError> {
    if adding <= 0 {
        return Ok(());
    }
    let Some(row) = sqlx::query(
        "SELECT t.channel_id, c.workspace_id FROM maidan_threads t
         JOIN maidan_channels c ON c.id = t.channel_id WHERE t.id = ?",
    )
    .bind(thread_id.0)
    .fetch_optional(pool)
    .await?
    else {
        return Ok(());
    };
    let channel_id = ChannelId(row.get::<Uuid, _>("channel_id"));
    let workspace_id = WorkspaceId(row.get::<Uuid, _>("workspace_id"));
    let Some(budget) = super::spawn::get_budget(pool, workspace_id).await? else {
        return Ok(());
    };
    if let Some(max_tools) = budget.max_tools {
        let existing = super::spawn::count_tool_uses(pool, thread_id).await?;
        if existing + adding > max_tools {
            return Err(StoreError::spawn_rejected(SpawnDenial {
                workspace_id,
                channel_id,
                thread_id,
                axis: SpawnAxis::Tools,
                limit: max_tools,
                observed: existing,
            }));
        }
    }
    Ok(())
}

pub async fn create(pool: &SqlitePool, new: NewMessage) -> Result<Message, StoreError> {
    enforce_tool_budget(pool, new.thread_id, new_tool_uses(&new.content)).await?;
    let id = Uuid::now_v7();
    let now = Utc::now();
    let metadata_text = serde_json::to_string(&new.metadata)?;
    let content_text = new
        .content
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?;
    let row = sqlx::query(
        "INSERT INTO maidan_messages (id, thread_id, author_id, body, metadata, content, posted_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         RETURNING id, thread_id, author_id, body, metadata, content, posted_at, edited_at, tombstoned_at",
    )
    .bind(id)
    .bind(new.thread_id.0)
    .bind(new.author_id.0)
    .bind(&new.body)
    .bind(&metadata_text)
    .bind(&content_text)
    .bind(now)
    .fetch_one(pool)
    .await?;
    row_to_message(&row)
}

/// Insert a message and append its `MessagePosted` event in one transaction.
/// Used by the DM / group-DM post paths, which — unlike the regular route — do
/// no post-insert slash-command edit, so the event reflects the final message.
/// `dm_conversation_id` is `Some` for a 1:1 DM, `None` for a group DM (matching
/// the pre-migration events).
pub async fn create_with_event(
    pool: &SqlitePool,
    new: NewMessage,
    dm_conversation_id: Option<DmConversationId>,
) -> Result<(Message, StoredEvent), StoreError> {
    enforce_tool_budget(pool, new.thread_id, new_tool_uses(&new.content)).await?;
    let id = Uuid::now_v7();
    let now = Utc::now();
    let metadata_text = serde_json::to_string(&new.metadata)?;
    let content_text = new
        .content
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?;
    let mut tx = pool.begin().await?;
    let row = sqlx::query(
        "INSERT INTO maidan_messages (id, thread_id, author_id, body, metadata, content, posted_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         RETURNING id, thread_id, author_id, body, metadata, content, posted_at, edited_at, tombstoned_at",
    )
    .bind(id)
    .bind(new.thread_id.0)
    .bind(new.author_id.0)
    .bind(&new.body)
    .bind(&metadata_text)
    .bind(&content_text)
    .bind(now)
    .fetch_one(&mut *tx)
    .await?;
    let message = row_to_message(&row)?;
    let (workspace_id, channel_id, thread_id) =
        events::message_scope_in_tx(&mut tx, message.id).await?;
    let event = Event::MessagePosted {
        occurred_at: Utc::now(),
        workspace_id,
        channel_id,
        thread_id,
        dm_conversation_id,
        message: message.clone(),
    };
    let stored = events::append_in_tx(&mut tx, &event).await?;
    // A post bumps its thread's activity clock (`updated_at`) so a
    // recently-active view can float it to the top. In-tx, atomic with the
    // post.
    sqlx::query("UPDATE maidan_threads SET updated_at = ? WHERE id = ?")
        .bind(chrono::Utc::now().to_rfc3339())
        .bind(thread_id.0)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok((message, stored))
}

pub async fn get(pool: &SqlitePool, id: MessageId) -> Result<Message, StoreError> {
    let row = sqlx::query(
        "SELECT id, thread_id, author_id, body, metadata, content, posted_at, edited_at, tombstoned_at
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
        "SELECT id, thread_id, author_id, body, metadata, content, posted_at, edited_at, tombstoned_at
         FROM maidan_messages
         WHERE thread_id = ? AND tombstoned_at IS NULL
         ORDER BY posted_at ASC, id ASC
         LIMIT ?",
    )
    .bind(thread_id.0)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_message).collect()
}

pub async fn list_after(
    pool: &SqlitePool,
    thread_id: ThreadId,
    after: Option<MessageId>,
    limit: i64,
) -> Result<Vec<Message>, StoreError> {
    let rows = match after {
        None => {
            sqlx::query(
                "SELECT id, thread_id, author_id, body, metadata, content, posted_at, edited_at, tombstoned_at
                 FROM maidan_messages
                 WHERE thread_id = ? AND tombstoned_at IS NULL
                 ORDER BY posted_at ASC, id ASC
                 LIMIT ?",
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
                 JOIN maidan_messages anchor ON anchor.id = ?
                 WHERE m.thread_id = ? AND m.tombstoned_at IS NULL
                   AND (m.posted_at > anchor.posted_at
                        OR (m.posted_at = anchor.posted_at AND m.id > anchor.id))
                 ORDER BY m.posted_at ASC, m.id ASC
                 LIMIT ?",
            )
            .bind(after_id.0)
            .bind(thread_id.0)
            .bind(limit)
            .fetch_all(pool)
            .await?
        }
    };
    rows.iter().map(row_to_message).collect()
}

pub async fn purge(pool: &SqlitePool, id: MessageId) -> Result<(), StoreError> {
    let mut conn = pool.acquire().await?;
    purge_on(&mut conn, id).await
}

/// Remove a tombstoned message outright, refusing one in a workspace under
/// legal hold: a purge destroys the record, which is what a hold preserves.
pub(crate) async fn purge_on(conn: &mut SqliteConnection, id: MessageId) -> Result<(), StoreError> {
    let mut tx = sqlx::Connection::begin(&mut *conn).await?;
    let workspace_id: Option<Uuid> = sqlx::query_scalar(
        "SELECT c.workspace_id FROM maidan_messages m
         INNER JOIN maidan_threads t ON m.thread_id = t.id
         INNER JOIN maidan_channels c ON t.channel_id = c.id
         WHERE m.id = ?",
    )
    .bind(id.0)
    .fetch_optional(&mut *tx)
    .await?;
    let workspace_id = workspace_id.ok_or(StoreError::NotFound)?;
    super::legal_hold::refuse_if_held(&mut tx, WorkspaceId(workspace_id)).await?;
    let res = sqlx::query("DELETE FROM maidan_messages WHERE id = ? AND tombstoned_at IS NOT NULL")
        .bind(id.0)
        .execute(&mut *tx)
        .await?;
    if res.rows_affected() == 0 {
        return Err(StoreError::NotFound);
    }
    tx.commit().await?;
    Ok(())
}

pub async fn edit(
    pool: &SqlitePool,
    id: MessageId,
    editor_id: MemberId,
    edit: EditMessage,
) -> Result<Message, StoreError> {
    let existing = get(pool, id).await?;
    let now = Utc::now();
    if existing.body != edit.body {
        super::message_edits::append(pool, id, editor_id, &existing.body, &edit.body, now).await?;
    }
    let metadata_text = serde_json::to_string(&edit.metadata)?;
    let content_text = edit
        .content
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?;
    let row = sqlx::query(
        "UPDATE maidan_messages SET body = ?, metadata = ?, content = ?, edited_at = ?
         WHERE id = ? AND tombstoned_at IS NULL
         RETURNING id, thread_id, author_id, body, metadata, content, posted_at, edited_at, tombstoned_at",
    )
    .bind(&edit.body)
    .bind(&metadata_text)
    .bind(&content_text)
    .bind(now)
    .bind(id.0)
    .fetch_optional(pool)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_message(&row)
}

/// The edit mutation on a caller-supplied tx, without committing. Shared by
/// `edit_with_event` (emits `MessageEdited`) and `edit_with_posted_event`
/// (emits `MessagePosted` for the slash finalization) — records the
/// edit-history row when the body changes, then updates and returns the
/// message.
async fn edit_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    id: MessageId,
    editor_id: MemberId,
    edit: &EditMessage,
) -> Result<Message, StoreError> {
    let existing_row = sqlx::query(
        "SELECT id, thread_id, author_id, body, metadata, content, posted_at, edited_at, tombstoned_at
         FROM maidan_messages WHERE id = ?",
    )
    .bind(id.0)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(StoreError::NotFound)?;
    let existing = row_to_message(&existing_row)?;
    let now = Utc::now();
    if existing.body != edit.body {
        super::message_edits::append_in_tx(tx, id, editor_id, &existing.body, &edit.body, now)
            .await?;
    }
    let metadata_text = serde_json::to_string(&edit.metadata)?;
    let content_text = edit
        .content
        .as_ref()
        .map(serde_json::to_string)
        .transpose()?;
    let row = sqlx::query(
        "UPDATE maidan_messages SET body = ?, metadata = ?, content = ?, edited_at = ?
         WHERE id = ? AND tombstoned_at IS NULL
         RETURNING id, thread_id, author_id, body, metadata, content, posted_at, edited_at, tombstoned_at",
    )
    .bind(&edit.body)
    .bind(&metadata_text)
    .bind(&content_text)
    .bind(now)
    .bind(id.0)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(StoreError::NotFound)?;
    row_to_message(&row)
}

/// Edit a message and append its `MessageEdited` event in one transaction.
pub async fn edit_with_event(
    pool: &SqlitePool,
    id: MessageId,
    editor_id: MemberId,
    edit: EditMessage,
    dm_conversation_id: Option<DmConversationId>,
) -> Result<(Message, StoredEvent), StoreError> {
    let mut tx = pool.begin().await?;
    let message = edit_in_tx(&mut tx, id, editor_id, &edit).await?;
    let (workspace_id, channel_id, thread_id) =
        events::message_scope_in_tx(&mut tx, message.id).await?;
    let event = Event::MessageEdited {
        occurred_at: Utc::now(),
        workspace_id,
        channel_id,
        thread_id,
        dm_conversation_id,
        editor_id,
        message: message.clone(),
    };
    let stored = events::append_in_tx(&mut tx, &event).await?;
    tx.commit().await?;
    Ok((message, stored))
}

/// Edit a just-posted message and append its `MessagePosted` event reflecting
/// the
/// **edited** message, in one transaction. The atomic tail of the
/// regular message-post path's slash finalization — the event carries the
/// post-slash message.
pub async fn edit_with_posted_event(
    pool: &SqlitePool,
    id: MessageId,
    editor_id: MemberId,
    edit: EditMessage,
    dm_conversation_id: Option<DmConversationId>,
) -> Result<(Message, StoredEvent), StoreError> {
    let mut tx = pool.begin().await?;
    let message = edit_in_tx(&mut tx, id, editor_id, &edit).await?;
    let (workspace_id, channel_id, thread_id) =
        events::message_scope_in_tx(&mut tx, message.id).await?;
    let event = Event::MessagePosted {
        occurred_at: Utc::now(),
        workspace_id,
        channel_id,
        thread_id,
        dm_conversation_id,
        message: message.clone(),
    };
    let stored = events::append_in_tx(&mut tx, &event).await?;
    // A post bumps its thread's activity clock (`updated_at`) so a
    // recently-active view can float it to the top. In-tx, atomic with the
    // post.
    sqlx::query("UPDATE maidan_threads SET updated_at = ? WHERE id = ?")
        .bind(chrono::Utc::now().to_rfc3339())
        .bind(thread_id.0)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok((message, stored))
}

pub async fn tombstone(pool: &SqlitePool, id: MessageId) -> Result<(), StoreError> {
    let mut tx = pool.begin().await?;
    super::legal_hold::preserve_or_forget(&mut tx, id).await?;
    crate::embeddings_purge::purge_message_embeddings_sqlite(&mut tx, id).await?;
    let now = Utc::now();
    let res = sqlx::query(
        "UPDATE maidan_messages SET tombstoned_at = ?, body = '', content = NULL WHERE id = ? AND tombstoned_at IS NULL",
    )
    .bind(now)
    .bind(id.0)
    .execute(&mut *tx)
    .await?;
    if res.rows_affected() == 0 {
        return Err(StoreError::NotFound);
    }
    tx.commit().await?;
    Ok(())
}

/// Tombstone a message and append its `MessageTombstoned` event in one
/// transaction. `NotFound` if already tombstoned or absent.
pub async fn tombstone_with_event(
    pool: &SqlitePool,
    id: MessageId,
    dm_conversation_id: Option<DmConversationId>,
) -> Result<StoredEvent, StoreError> {
    let mut tx = pool.begin().await?;
    super::legal_hold::preserve_or_forget(&mut tx, id).await?;
    crate::embeddings_purge::purge_message_embeddings_sqlite(&mut tx, id).await?;
    let now = Utc::now();
    let res = sqlx::query(
        "UPDATE maidan_messages SET tombstoned_at = ?, body = '', content = NULL WHERE id = ? AND tombstoned_at IS NULL",
    )
    .bind(now)
    .bind(id.0)
    .execute(&mut *tx)
    .await?;
    if res.rows_affected() == 0 {
        return Err(StoreError::NotFound);
    }
    let (workspace_id, channel_id, thread_id) = events::message_scope_in_tx(&mut tx, id).await?;
    let event = Event::MessageTombstoned {
        occurred_at: Utc::now(),
        workspace_id,
        channel_id,
        thread_id,
        dm_conversation_id,
        message_id: id,
    };
    let stored = events::append_in_tx(&mut tx, &event).await?;
    tx.commit().await?;
    Ok(stored)
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
        content: row
            .get::<Option<String>, _>("content")
            .and_then(|s| serde_json::from_str(&s).ok()),
        posted_at: row.get::<DateTime<Utc>, _>("posted_at"),
        edited_at: row.get::<Option<DateTime<Utc>>, _>("edited_at"),
        tombstoned_at: row.get::<Option<DateTime<Utc>>, _>("tombstoned_at"),
    })
}

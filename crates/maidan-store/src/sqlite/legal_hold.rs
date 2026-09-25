//! Legal-hold queries: the `maidan_legal_holds` table. A workspace with a row
//! here is under hold; the retention prune SQL (`retention.rs`) reads this
//! table directly to exempt held workspaces' events and to freeze audit
//! pruning.

use chrono::{DateTime, Utc};
use maidan_types::{
    ChannelId, LegalHold, MemberId, MessageId, PreservedMessage, ThreadId, WorkspaceId,
};
use sqlx::{Row, SqliteConnection, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;

fn row_to_hold(row: &sqlx::sqlite::SqliteRow) -> LegalHold {
    LegalHold {
        workspace_id: WorkspaceId(row.get::<Uuid, _>("workspace_id")),
        reason: row.get::<String, _>("reason"),
        placed_by: row.get::<Option<Uuid>, _>("placed_by").map(MemberId),
        placed_at: row.get::<DateTime<Utc>, _>("placed_at"),
    }
}

const COLS: &str = "workspace_id, reason, placed_by, placed_at";

pub async fn place(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    reason: &str,
    placed_by: Option<MemberId>,
) -> Result<LegalHold, StoreError> {
    let mut conn = pool.acquire().await?;
    place_on(&mut conn, workspace_id, reason, placed_by).await
}

pub(crate) async fn place_on(
    conn: &mut SqliteConnection,
    workspace_id: WorkspaceId,
    reason: &str,
    placed_by: Option<MemberId>,
) -> Result<LegalHold, StoreError> {
    let now = Utc::now().to_rfc3339();
    let row = sqlx::query(&format!(
        "INSERT INTO maidan_legal_holds (workspace_id, reason, placed_by, placed_at)
         VALUES (?, ?, ?, ?)
         ON CONFLICT (workspace_id) DO UPDATE SET
             reason = excluded.reason,
             placed_by = excluded.placed_by,
             placed_at = excluded.placed_at
         RETURNING {COLS}"
    ))
    .bind(workspace_id.0)
    .bind(reason)
    .bind(placed_by.map(|m| m.0))
    .bind(&now)
    .fetch_one(&mut *conn)
    .await?;
    Ok(row_to_hold(&row))
}

pub async fn lift(pool: &SqlitePool, workspace_id: WorkspaceId) -> Result<bool, StoreError> {
    let mut tx = pool.begin().await?;
    let lifted = lift_on(&mut tx, workspace_id).await?;
    tx.commit().await?;
    Ok(lifted.is_some())
}

/// Lift the hold and dispose of what it kept. `None` when there was no hold.
/// Run in a transaction: the disposal and the lift stand or fall together.
pub(crate) async fn lift_on(
    conn: &mut SqliteConnection,
    workspace_id: WorkspaceId,
) -> Result<Option<crate::HoldDisposal>, StoreError> {
    let done = sqlx::query("DELETE FROM maidan_legal_holds WHERE workspace_id = ?")
        .bind(workspace_id.0)
        .execute(&mut *conn)
        .await?;
    if done.rows_affected() == 0 {
        return Ok(None);
    }
    // Earlier versions of messages withdrawn while held, then their last words.
    let edit_versions = sqlx::query(
        "DELETE FROM maidan_message_edits WHERE message_id IN (
             SELECT m.id FROM maidan_messages m
             JOIN maidan_threads t ON t.id = m.thread_id
             JOIN maidan_channels c ON c.id = t.channel_id
             WHERE c.workspace_id = ? AND m.tombstoned_at IS NOT NULL
         )",
    )
    .bind(workspace_id.0)
    .execute(&mut *conn)
    .await?
    .rows_affected();
    let withdrawn_messages =
        sqlx::query("DELETE FROM maidan_preserved_messages WHERE workspace_id = ?")
            .bind(workspace_id.0)
            .execute(&mut *conn)
            .await?
            .rows_affected();
    Ok(Some(crate::HoldDisposal {
        withdrawn_messages,
        edit_versions,
    }))
}

/// Before a message is withdrawn: under a hold, keep its words; otherwise
/// forget its earlier versions, so the withdrawal withdraws. Run in the
/// tombstoning transaction, before the row is blanked. `NotFound` when the
/// message is absent or already withdrawn.
pub(crate) async fn preserve_or_forget(
    conn: &mut SqliteConnection,
    message_id: MessageId,
) -> Result<(), StoreError> {
    let workspace_id: Option<Uuid> = sqlx::query_scalar(
        "SELECT c.workspace_id FROM maidan_messages m
         JOIN maidan_threads t ON t.id = m.thread_id
         JOIN maidan_channels c ON c.id = t.channel_id
         WHERE m.id = ? AND m.tombstoned_at IS NULL",
    )
    .bind(message_id.0)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(workspace_id) = workspace_id else {
        return Err(StoreError::NotFound);
    };
    let held: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM maidan_legal_holds WHERE workspace_id = ?)",
    )
    .bind(workspace_id)
    .fetch_one(&mut *conn)
    .await?;
    if held {
        sqlx::query(
            "INSERT INTO maidan_preserved_messages
                 (message_id, workspace_id, body, content, tombstoned_at)
             SELECT id, ?, body, content, ? FROM maidan_messages WHERE id = ?
             ON CONFLICT (message_id) DO NOTHING",
        )
        .bind(workspace_id)
        .bind(Utc::now().to_rfc3339())
        .bind(message_id.0)
        .execute(&mut *conn)
        .await?;
    } else {
        sqlx::query("DELETE FROM maidan_message_edits WHERE message_id = ?")
            .bind(message_id.0)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

/// Every message the workspace's hold has kept, newest withdrawal first, with
/// its earlier versions.
pub(crate) async fn preserved_on(
    conn: &mut SqliteConnection,
    workspace_id: WorkspaceId,
) -> Result<Vec<PreservedMessage>, StoreError> {
    let rows = sqlx::query(
        "SELECT p.message_id, m.thread_id, t.channel_id, m.author_id, m.posted_at,
                p.body, p.content, p.tombstoned_at
         FROM maidan_preserved_messages p
         JOIN maidan_messages m ON m.id = p.message_id
         JOIN maidan_threads t ON t.id = m.thread_id
         WHERE p.workspace_id = ?
         ORDER BY p.tombstoned_at DESC, p.message_id",
    )
    .bind(workspace_id.0)
    .fetch_all(&mut *conn)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        let message_id = MessageId(row.get::<Uuid, _>("message_id"));
        let edits = sqlx::query(
            "SELECT id, message_id, editor_id, body_before, body_after, edited_at
             FROM maidan_message_edits WHERE message_id = ?
             ORDER BY edited_at ASC, id ASC",
        )
        .bind(message_id.0)
        .fetch_all(&mut *conn)
        .await?
        .iter()
        .map(super::message_edits::row_to_edit)
        .collect::<Result<Vec<_>, _>>()?;
        out.push(PreservedMessage {
            message_id,
            thread_id: ThreadId(row.get::<Uuid, _>("thread_id")),
            channel_id: ChannelId(row.get::<Uuid, _>("channel_id")),
            author_id: MemberId(row.get::<Uuid, _>("author_id")),
            posted_at: row.get::<DateTime<Utc>, _>("posted_at"),
            body: row.get("body"),
            content: row
                .get::<Option<String>, _>("content")
                .and_then(|s| serde_json::from_str(&s).ok()),
            tombstoned_at: row.get::<DateTime<Utc>, _>("tombstoned_at"),
            edits,
        });
    }
    Ok(out)
}

/// Refuse to destroy a held workspace's data: `Conflict`, which the API
/// answers with 409. `NotFound` when the workspace does not exist. Run it in
/// the destroying transaction.
pub(crate) async fn refuse_if_held(
    conn: &mut SqliteConnection,
    workspace_id: WorkspaceId,
) -> Result<(), StoreError> {
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM maidan_workspaces WHERE id = ?)")
            .bind(workspace_id.0)
            .fetch_one(&mut *conn)
            .await?;
    if !exists {
        return Err(StoreError::NotFound);
    }
    let held: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM maidan_legal_holds WHERE workspace_id = ?)",
    )
    .bind(workspace_id.0)
    .fetch_one(&mut *conn)
    .await?;
    if held {
        return Err(StoreError::Conflict(crate::LEGAL_HOLD_REFUSAL.into()));
    }
    Ok(())
}

pub async fn get(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
) -> Result<Option<LegalHold>, StoreError> {
    let row = sqlx::query(&format!(
        "SELECT {COLS} FROM maidan_legal_holds WHERE workspace_id = ?"
    ))
    .bind(workspace_id.0)
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(row_to_hold))
}

pub async fn list(pool: &SqlitePool) -> Result<Vec<LegalHold>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT {COLS} FROM maidan_legal_holds ORDER BY placed_at DESC, workspace_id ASC"
    ))
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_hold).collect())
}

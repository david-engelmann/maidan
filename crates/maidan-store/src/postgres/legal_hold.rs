//! Legal-hold queries: the `maidan_legal_holds` table. A workspace with a row
//! here is under hold; the retention prune SQL (`retention.rs`) reads this
//! table directly to exempt held workspaces' events and to freeze audit
//! pruning.

use chrono::{DateTime, Utc};
use maidan_types::{
    ChannelId, LegalHold, MemberId, MessageId, PreservedMessage, ThreadId, WorkspaceId,
};
use sqlx::{PgConnection, PgPool, Row};
use uuid::Uuid;

use crate::error::StoreError;

fn row_to_hold(row: &sqlx::postgres::PgRow) -> LegalHold {
    LegalHold {
        workspace_id: WorkspaceId(row.get::<Uuid, _>("workspace_id")),
        reason: row.get::<String, _>("reason"),
        placed_by: row.get::<Option<Uuid>, _>("placed_by").map(MemberId),
        placed_at: row.get::<DateTime<Utc>, _>("placed_at"),
    }
}

const COLS: &str = "workspace_id, reason, placed_by, placed_at";

pub async fn place(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    reason: &str,
    placed_by: Option<MemberId>,
) -> Result<LegalHold, StoreError> {
    let mut conn = pool.acquire().await?;
    place_on(&mut conn, workspace_id, reason, placed_by).await
}

pub(crate) async fn place_on(
    conn: &mut PgConnection,
    workspace_id: WorkspaceId,
    reason: &str,
    placed_by: Option<MemberId>,
) -> Result<LegalHold, StoreError> {
    // Serialize with a purge or erase of the same workspace, which takes this
    // lock before checking for a hold (`refuse_if_held`).
    sqlx::query("SELECT 1 FROM maidan_workspaces WHERE id = $1 FOR NO KEY UPDATE")
        .bind(workspace_id.0)
        .fetch_optional(&mut *conn)
        .await?;
    let row = sqlx::query(&format!(
        "INSERT INTO maidan_legal_holds (workspace_id, reason, placed_by, placed_at)
         VALUES ($1, $2, $3, NOW())
         ON CONFLICT (workspace_id) DO UPDATE SET
             reason = excluded.reason,
             placed_by = excluded.placed_by,
             placed_at = excluded.placed_at
         RETURNING {COLS}"
    ))
    .bind(workspace_id.0)
    .bind(reason)
    .bind(placed_by.map(|m| m.0))
    .fetch_one(&mut *conn)
    .await?;
    Ok(row_to_hold(&row))
}

pub async fn lift(pool: &PgPool, workspace_id: WorkspaceId) -> Result<bool, StoreError> {
    let mut tx = pool.begin().await?;
    let lifted = lift_on(&mut tx, workspace_id).await?;
    tx.commit().await?;
    Ok(lifted.is_some())
}

/// Lift the hold and dispose of what it kept (SQLite twin). `None` when there
/// was no hold.
pub(crate) async fn lift_on(
    conn: &mut PgConnection,
    workspace_id: WorkspaceId,
) -> Result<Option<crate::HoldDisposal>, StoreError> {
    let done = sqlx::query("DELETE FROM maidan_legal_holds WHERE workspace_id = $1")
        .bind(workspace_id.0)
        .execute(&mut *conn)
        .await?;
    if done.rows_affected() == 0 {
        return Ok(None);
    }
    let edit_versions = sqlx::query(
        "DELETE FROM maidan_message_edits e
         USING maidan_messages m, maidan_threads t, maidan_channels c
         WHERE e.message_id = m.id AND t.id = m.thread_id AND c.id = t.channel_id
           AND c.workspace_id = $1 AND m.tombstoned_at IS NOT NULL",
    )
    .bind(workspace_id.0)
    .execute(&mut *conn)
    .await?
    .rows_affected();
    let withdrawn_messages =
        sqlx::query("DELETE FROM maidan_preserved_messages WHERE workspace_id = $1")
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
/// forget its earlier versions (SQLite twin). The hold row is locked for
/// share, so a lift waits for this withdrawal and then disposes of what it
/// kept.
pub(crate) async fn preserve_or_forget(
    conn: &mut PgConnection,
    message_id: MessageId,
) -> Result<(), StoreError> {
    let workspace_id: Option<Uuid> = sqlx::query_scalar(
        "SELECT c.workspace_id FROM maidan_messages m
         JOIN maidan_threads t ON t.id = m.thread_id
         JOIN maidan_channels c ON c.id = t.channel_id
         WHERE m.id = $1 AND m.tombstoned_at IS NULL",
    )
    .bind(message_id.0)
    .fetch_optional(&mut *conn)
    .await?;
    let Some(workspace_id) = workspace_id else {
        return Err(StoreError::NotFound);
    };
    let held = sqlx::query("SELECT 1 FROM maidan_legal_holds WHERE workspace_id = $1 FOR SHARE")
        .bind(workspace_id)
        .fetch_optional(&mut *conn)
        .await?
        .is_some();
    if held {
        sqlx::query(
            "INSERT INTO maidan_preserved_messages
                 (message_id, workspace_id, body, content, tombstoned_at)
             SELECT id, $2, body, content, NOW() FROM maidan_messages WHERE id = $1
             ON CONFLICT (message_id) DO NOTHING",
        )
        .bind(message_id.0)
        .bind(workspace_id)
        .execute(&mut *conn)
        .await?;
    } else {
        sqlx::query("DELETE FROM maidan_message_edits WHERE message_id = $1")
            .bind(message_id.0)
            .execute(&mut *conn)
            .await?;
    }
    Ok(())
}

/// Every message the workspace's hold has kept (SQLite twin).
pub(crate) async fn preserved_on(
    conn: &mut PgConnection,
    workspace_id: WorkspaceId,
) -> Result<Vec<PreservedMessage>, StoreError> {
    let rows = sqlx::query(
        "SELECT p.message_id, m.thread_id, t.channel_id, m.author_id, m.posted_at,
                p.body, p.content, p.tombstoned_at
         FROM maidan_preserved_messages p
         JOIN maidan_messages m ON m.id = p.message_id
         JOIN maidan_threads t ON t.id = m.thread_id
         WHERE p.workspace_id = $1
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
             FROM maidan_message_edits WHERE message_id = $1
             ORDER BY edited_at ASC, id ASC",
        )
        .bind(message_id.0)
        .fetch_all(&mut *conn)
        .await?
        .iter()
        .map(super::message_edits::row_to_edit)
        .collect();
        out.push(PreservedMessage {
            message_id,
            thread_id: ThreadId(row.get::<Uuid, _>("thread_id")),
            channel_id: ChannelId(row.get::<Uuid, _>("channel_id")),
            author_id: MemberId(row.get::<Uuid, _>("author_id")),
            posted_at: row.get::<DateTime<Utc>, _>("posted_at"),
            body: row.get("body"),
            content: row
                .get::<Option<serde_json::Value>, _>("content")
                .and_then(|v| serde_json::from_value(v).ok()),
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
    conn: &mut PgConnection,
    workspace_id: WorkspaceId,
) -> Result<(), StoreError> {
    // Held until the transaction ends, so a hold placed meanwhile waits for it
    // (see `place_on`).
    let exists = sqlx::query("SELECT 1 FROM maidan_workspaces WHERE id = $1 FOR NO KEY UPDATE")
        .bind(workspace_id.0)
        .fetch_optional(&mut *conn)
        .await?
        .is_some();
    if !exists {
        return Err(StoreError::NotFound);
    }
    let held: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM maidan_legal_holds WHERE workspace_id = $1)",
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
    pool: &PgPool,
    workspace_id: WorkspaceId,
) -> Result<Option<LegalHold>, StoreError> {
    let row = sqlx::query(&format!(
        "SELECT {COLS} FROM maidan_legal_holds WHERE workspace_id = $1"
    ))
    .bind(workspace_id.0)
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(row_to_hold))
}

pub async fn list(pool: &PgPool) -> Result<Vec<LegalHold>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT {COLS} FROM maidan_legal_holds ORDER BY placed_at DESC, workspace_id ASC"
    ))
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_hold).collect())
}

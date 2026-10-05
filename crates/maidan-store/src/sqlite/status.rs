//! Agent self-reported status: the `maidan_thread_status` side table.
//! Presence = an active declaration; absence = cleared. `stalled` is
//! system-computed only and can never be declared (enforced by
//! [`DeclaredStatus::parse`] refusing it).

use chrono::{DateTime, Utc};
use maidan_types::{
    DeclaredStatus, Event, MemberId, StoredEvent, ThreadId, ThreadStatusDeclaration,
};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;
use crate::sqlite::events;

fn row_to_declaration(
    row: &sqlx::sqlite::SqliteRow,
) -> Result<ThreadStatusDeclaration, StoreError> {
    let raw: String = row.get("status");
    let status = DeclaredStatus::parse(&raw)
        .ok_or_else(|| StoreError::InvalidInput(format!("unknown declared status: {raw}")))?;
    Ok(ThreadStatusDeclaration {
        thread_id: ThreadId(row.get::<Uuid, _>("thread_id")),
        status,
        note: row.get("note"),
        declared_by: MemberId(row.get::<Uuid, _>("declared_by")),
        declared_at: row.get::<DateTime<Utc>, _>("declared_at"),
    })
}

/// Declare (or supersede) the agent's status on a thread and append
/// `StatusDeclared` in one tx. Returns the declaration and the stored event.
pub async fn declare(
    pool: &SqlitePool,
    thread_id: ThreadId,
    status: DeclaredStatus,
    note: String,
    declared_by: MemberId,
) -> Result<(ThreadStatusDeclaration, StoredEvent), StoreError> {
    let mut tx = pool.begin().await?;
    let now = Utc::now().to_rfc3339();
    let row = sqlx::query(
        "INSERT INTO maidan_thread_status (thread_id, status, note, declared_by, declared_at)
         VALUES (?, ?, ?, ?, ?)
         ON CONFLICT (thread_id) DO UPDATE SET
             status = excluded.status, note = excluded.note,
             declared_by = excluded.declared_by, declared_at = excluded.declared_at
         RETURNING thread_id, status, note, declared_by, declared_at",
    )
    .bind(thread_id.0)
    .bind(status.as_str())
    .bind(&note)
    .bind(declared_by.0)
    .bind(&now)
    .fetch_one(&mut *tx)
    .await?;
    let declaration = row_to_declaration(&row)?;
    let (workspace_id, channel_id) = events::thread_scope_in_tx(&mut tx, thread_id).await?;
    let event = Event::StatusDeclared {
        occurred_at: Utc::now(),
        workspace_id,
        channel_id,
        thread_id,
        status,
        note: note.clone(),
        declared_by,
    };
    let stored = events::append_in_tx(&mut tx, &event).await?;
    tx.commit().await?;
    Ok((declaration, stored))
}

/// Clear the declaration without an event. Used when a human responds —
/// the declaration is superseded by human activity, not by a status change.
pub async fn clear(
    pool: &SqlitePool,
    thread_id: ThreadId,
) -> Result<Option<ThreadStatusDeclaration>, StoreError> {
    let row = sqlx::query(
        "DELETE FROM maidan_thread_status WHERE thread_id = ?
         RETURNING thread_id, status, note, declared_by, declared_at",
    )
    .bind(thread_id.0)
    .fetch_optional(pool)
    .await?;
    row.as_ref().map(row_to_declaration).transpose()
}

pub async fn get(
    pool: &SqlitePool,
    thread_id: ThreadId,
) -> Result<Option<ThreadStatusDeclaration>, StoreError> {
    let row = sqlx::query(
        "SELECT thread_id, status, note, declared_by, declared_at
         FROM maidan_thread_status WHERE thread_id = ?",
    )
    .bind(thread_id.0)
    .fetch_optional(pool)
    .await?;
    row.as_ref().map(row_to_declaration).transpose()
}

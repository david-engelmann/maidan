//! Agent self-reported status: the `maidan_thread_status` side table.
//! Presence = an active declaration; absence = cleared. `stalled` is
//! system-computed only and can never be declared (the type has no `Stalled`
//! variant, so serde refuses it; [`DeclaredStatus::parse`] is only used for
//! DB reads).

use chrono::{DateTime, Utc};
use maidan_types::{
    ChannelId, DeclaredStatus, Event, MemberId, StoredEvent, ThreadId, ThreadStatusDeclaration,
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
///
/// The declaration is by the claim holder or the thread owner; anyone else
/// is refused. The check runs inside the declaration transaction. Unlike the
/// Postgres path this SELECT takes no row lock — SQLite has no `FOR UPDATE` —
/// so a claim handoff committing between the check and the write is evaluated
/// against the pre-handoff holder. The window is one statement wide; the
/// Postgres path closes it with a row lock.
pub async fn declare(
    pool: &SqlitePool,
    thread_id: ThreadId,
    status: DeclaredStatus,
    note: String,
    declared_by: MemberId,
) -> Result<(ThreadStatusDeclaration, StoredEvent), StoreError> {
    let mut tx = pool.begin().await?;
    // Who may speak for the thread: the member holding its claim
    // (`assignee_id`), or the thread's owner. Anyone else gets a refusal,
    // not a silent overwrite of another agent's status.
    let holder = sqlx::query("SELECT assignee_id, owner_id FROM maidan_threads WHERE id = ?")
        .bind(thread_id.0)
        .fetch_optional(&mut *tx)
        .await?;
    let Some(row) = holder else {
        return Err(StoreError::NotFound);
    };
    let assignee: Option<Uuid> = row.get("assignee_id");
    let owner: Option<Uuid> = row.get("owner_id");
    if assignee != Some(declared_by.0) && owner != Some(declared_by.0) {
        return Err(StoreError::Conflict(
            "only the claim holder or the thread owner may declare status".into(),
        ));
    }
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

/// Active declarations for a channel's live threads, for the board's thread
/// list enrichment. One read, not per-thread.
pub async fn list_for_channel(
    pool: &SqlitePool,
    channel_id: ChannelId,
) -> Result<Vec<ThreadStatusDeclaration>, StoreError> {
    let rows = sqlx::query(
        "SELECT s.thread_id, s.status, s.note, s.declared_by, s.declared_at
         FROM maidan_thread_status s
         JOIN maidan_threads t ON t.id = s.thread_id
         WHERE t.channel_id = ? AND t.tombstoned_at IS NULL",
    )
    .bind(channel_id.0)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_declaration).collect()
}

/// Threads in a workspace whose agent declared `needs_input`, with their
/// titles, owners and declarations, oldest question first. For the waiting
/// inbox: an agent's question waits on a human until someone answers in the
/// thread, which clears the declaration.
pub async fn list_needs_input(
    pool: &SqlitePool,
    workspace_id: maidan_types::WorkspaceId,
) -> Result<
    Vec<(
        ThreadId,
        Option<String>,
        Option<MemberId>,
        ThreadStatusDeclaration,
    )>,
    StoreError,
> {
    let rows = sqlx::query(
        "SELECT t.title AS t_title, t.owner_id AS t_owner,
                s.thread_id, s.status, s.note, s.declared_by, s.declared_at
         FROM maidan_thread_status s
         JOIN maidan_threads t ON t.id = s.thread_id
         JOIN maidan_channels c ON c.id = t.channel_id
         JOIN maidan_members m ON m.id = s.declared_by AND m.kind = 'agent'
         WHERE c.workspace_id = ?
           AND s.status = 'needs_input'
           AND t.tombstoned_at IS NULL
           AND t.state NOT IN ('closed', 'archived')
         ORDER BY s.declared_at, s.thread_id",
    )
    .bind(workspace_id.0)
    .fetch_all(pool)
    .await?;
    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        let declaration = row_to_declaration(row)?;
        let title: Option<String> = row.get("t_title");
        let owner: Option<Uuid> = row.get("t_owner");
        out.push((
            declaration.thread_id,
            title,
            owner.map(MemberId),
            declaration,
        ));
    }
    Ok(out)
}

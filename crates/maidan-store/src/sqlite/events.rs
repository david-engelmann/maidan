use maidan_types::{ChannelId, Event, MessageId, StoredEvent, ThreadId, WorkspaceId};
use sqlx::{Row, SqlitePool};

use crate::error::StoreError;
use crate::sqlite::outbox;

/// Resolve a message's (workspace, channel, thread) inside a transaction
/// (Cluster 206) — used by the message-scoped `*_with_event` mutations to build
/// their event's context in the same tx as the mutation. `NotFound` if the
/// message doesn't exist.
pub async fn message_scope_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    message_id: MessageId,
) -> Result<(WorkspaceId, ChannelId, ThreadId), StoreError> {
    let row = sqlx::query(
        "SELECT c.workspace_id AS ws, t.channel_id AS ch, m.thread_id AS th
         FROM maidan_messages m
         JOIN maidan_threads t ON t.id = m.thread_id
         JOIN maidan_channels c ON c.id = t.channel_id
         WHERE m.id = ?",
    )
    .bind(message_id.0)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(StoreError::NotFound)?;
    Ok((
        WorkspaceId(row.get::<uuid::Uuid, _>("ws")),
        ChannelId(row.get::<uuid::Uuid, _>("ch")),
        ThreadId(row.get::<uuid::Uuid, _>("th")),
    ))
}

pub async fn append(pool: &SqlitePool, event: &Event) -> Result<StoredEvent, StoreError> {
    let mut tx = pool.begin().await?;
    let stored = append_in_tx(&mut tx, event).await?;
    tx.commit().await?;
    Ok(stored)
}

/// Append the event + its outbox row on a caller-supplied transaction, without
/// committing (Cluster 205 transactional outbox). A `*_with_event` store method
/// calls this in the SAME tx as its domain mutation so the domain row and the
/// event are committed atomically — a crash can no longer leave one without the
/// other.
pub async fn append_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    event: &Event,
) -> Result<StoredEvent, StoreError> {
    let payload = serde_json::to_string(event)?;
    // `inserted_at` is the DB insert wall-clock (Cluster 125 stability horizon),
    // distinct from the caller-supplied `occurred_at`.
    let row = sqlx::query(
        "INSERT INTO maidan_events (kind, workspace_id, channel_id, thread_id, payload, occurred_at, inserted_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         RETURNING id, kind, workspace_id, channel_id, thread_id, payload, occurred_at",
    )
    .bind(event.kind().as_str())
    .bind(event.workspace_id().map(|w| w.0))
    .bind(event.channel_id().map(|c| c.0))
    .bind(event.thread_id().map(|t| t.0))
    .bind(payload)
    .bind(event.occurred_at().to_rfc3339())
    .bind(chrono::Utc::now().to_rfc3339())
    .fetch_one(&mut **tx)
    .await?;
    let stored = row_to_stored(&row)?;
    outbox::enqueue_in_tx(tx, stored.id).await?;
    Ok(stored)
}

pub async fn get_by_id(pool: &SqlitePool, log_id: i64) -> Result<StoredEvent, StoreError> {
    let row = sqlx::query(
        "SELECT id, kind, workspace_id, channel_id, thread_id, payload, occurred_at
         FROM maidan_events
         WHERE id = ?",
    )
    .bind(log_id)
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else {
        return Err(StoreError::NotFound);
    };
    row_to_stored(&row)
}

pub async fn list_after(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    after_id: i64,
    limit: i64,
) -> Result<Vec<StoredEvent>, StoreError> {
    let rows = sqlx::query(
        "SELECT id, kind, workspace_id, channel_id, thread_id, payload, occurred_at
         FROM maidan_events
         WHERE workspace_id = ? AND id > ?
         ORDER BY id ASC
         LIMIT ?",
    )
    .bind(workspace_id.0)
    .bind(after_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_stored).collect()
}

/// Replay rows with `id > after_id` that are **stable** — inserted at or before
/// `stable_before` — in `id` order. Gating on `inserted_at` lets a reconcile
/// loop advance a durable cursor without stranding a lower `id` that is still
/// in flight (Cluster 125 at-least-once delivery). `inserted_at` is stored as
/// RFC3339, which sorts lexically in chronological order.
pub async fn list_after_stable(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    after_id: i64,
    stable_before: chrono::DateTime<chrono::Utc>,
    limit: i64,
) -> Result<Vec<StoredEvent>, StoreError> {
    let rows = sqlx::query(
        "SELECT id, kind, workspace_id, channel_id, thread_id, payload, occurred_at
         FROM maidan_events
         WHERE workspace_id = ? AND id > ? AND inserted_at <= ?
         ORDER BY id ASC
         LIMIT ?",
    )
    .bind(workspace_id.0)
    .bind(after_id)
    .bind(stable_before.to_rfc3339())
    .bind(limit)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_stored).collect()
}

fn row_to_stored(row: &sqlx::sqlite::SqliteRow) -> Result<StoredEvent, StoreError> {
    let kind_str: String = row.get("kind");
    let kind = parse_kind(&kind_str)?;
    let payload: String = row.get("payload");
    Ok(StoredEvent {
        id: row.get("id"),
        kind,
        workspace_id: row
            .get::<Option<uuid::Uuid>, _>("workspace_id")
            .map(maidan_types::WorkspaceId),
        channel_id: row
            .get::<Option<uuid::Uuid>, _>("channel_id")
            .map(maidan_types::ChannelId),
        thread_id: row
            .get::<Option<uuid::Uuid>, _>("thread_id")
            .map(maidan_types::ThreadId),
        payload: serde_json::from_str(&payload)?,
        occurred_at: row.get("occurred_at"),
    })
}

/// Parse the persisted `kind` column back into an [`EventKind`]. Delegates to
/// the single [`maidan_types::EventKind::parse`] so the wire-form mapping has no
/// per-backend copy to drift (Cluster 181 — Cluster 171 lost an event because a
/// store copy was missing a variant; the read-back failed and the insert rolled
/// back silently). Round-trip is guarded in `maidan-types`.
fn parse_kind(s: &str) -> Result<maidan_types::EventKind, StoreError> {
    maidan_types::EventKind::parse(s)
        .ok_or_else(|| StoreError::InvalidInput(format!("unknown event kind: {s}")))
}

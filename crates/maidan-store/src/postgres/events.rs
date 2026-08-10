use maidan_types::{Event, StoredEvent, WorkspaceId};
use sqlx::{PgPool, Row};

use crate::error::StoreError;
use crate::postgres::outbox;

pub async fn append(pool: &PgPool, event: &Event) -> Result<StoredEvent, StoreError> {
    let payload = serde_json::to_value(event)?;
    let mut tx = pool.begin().await?;
    // `inserted_at` is the DB insert wall-clock (Cluster 125 stability horizon),
    // distinct from the caller-supplied `occurred_at`.
    let row = sqlx::query(
        "INSERT INTO maidan_events (kind, workspace_id, channel_id, thread_id, payload, occurred_at, inserted_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         RETURNING id, kind, workspace_id, channel_id, thread_id, payload, occurred_at",
    )
    .bind(event.kind().as_str())
    .bind(event.workspace_id().map(|w| w.0))
    .bind(event.channel_id().map(|c| c.0))
    .bind(event.thread_id().map(|t| t.0))
    .bind(payload)
    .bind(event.occurred_at())
    .bind(chrono::Utc::now())
    .fetch_one(&mut *tx)
    .await?;
    let stored = row_to_stored(&row)?;
    outbox::enqueue_in_tx(&mut tx, stored.id).await?;
    tx.commit().await?;
    Ok(stored)
}

pub async fn get_by_id(pool: &PgPool, log_id: i64) -> Result<StoredEvent, StoreError> {
    let row = sqlx::query(
        "SELECT id, kind, workspace_id, channel_id, thread_id, payload, occurred_at
         FROM maidan_events
         WHERE id = $1",
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
    pool: &PgPool,
    workspace_id: WorkspaceId,
    after_id: i64,
    limit: i64,
) -> Result<Vec<StoredEvent>, StoreError> {
    let rows = sqlx::query(
        "SELECT id, kind, workspace_id, channel_id, thread_id, payload, occurred_at
         FROM maidan_events
         WHERE workspace_id = $1 AND id > $2
         ORDER BY id ASC
         LIMIT $3",
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
/// in flight (Cluster 125 at-least-once delivery).
pub async fn list_after_stable(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    after_id: i64,
    stable_before: chrono::DateTime<chrono::Utc>,
    limit: i64,
) -> Result<Vec<StoredEvent>, StoreError> {
    let rows = sqlx::query(
        "SELECT id, kind, workspace_id, channel_id, thread_id, payload, occurred_at
         FROM maidan_events
         WHERE workspace_id = $1 AND id > $2 AND inserted_at <= $3
         ORDER BY id ASC
         LIMIT $4",
    )
    .bind(workspace_id.0)
    .bind(after_id)
    .bind(stable_before)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_stored).collect()
}

fn row_to_stored(row: &sqlx::postgres::PgRow) -> Result<StoredEvent, StoreError> {
    let kind_str: String = row.get("kind");
    let kind = parse_kind(&kind_str)?;
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
        payload: row.get("payload"),
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

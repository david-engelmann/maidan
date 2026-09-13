//! Explicit dispatch-block queries (Cluster 386, Wave 2 #27, G14 + W2): the
//! `maidan_thread_blocks` side table. Presence = blocked from `claim_next`
//! with a closed [`BlockedReason`]; absence = unblocked. Distinct from
//! Cluster 217/218 DAG readiness and from Cluster 363's free-text park.

use chrono::{DateTime, Utc};
use maidan_types::{BlockedReason, ChannelId, Event, MemberId, StoredEvent, ThreadBlock, ThreadId};
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::StoreError;
use crate::postgres::events;

fn row_to_block(row: &sqlx::postgres::PgRow) -> Result<ThreadBlock, StoreError> {
    let raw: String = row.get("reason");
    let reason = BlockedReason::parse(&raw)
        .ok_or_else(|| StoreError::InvalidInput(format!("unknown blocked reason: {raw}")))?;
    Ok(ThreadBlock {
        thread_id: ThreadId(row.get::<Uuid, _>("thread_id")),
        reason,
        set_by: MemberId(row.get::<Uuid, _>("set_by")),
        set_at: row.get::<DateTime<Utc>, _>("set_at"),
    })
}

pub async fn set(
    pool: &PgPool,
    thread_id: ThreadId,
    reason: BlockedReason,
    set_by: MemberId,
) -> Result<ThreadBlock, StoreError> {
    let row = sqlx::query(
        "INSERT INTO maidan_thread_blocks (thread_id, reason, set_by)
         VALUES ($1, $2, $3)
         ON CONFLICT (thread_id) DO UPDATE SET
             reason = EXCLUDED.reason, set_by = EXCLUDED.set_by, set_at = NOW()
         RETURNING thread_id, reason, set_by, set_at",
    )
    .bind(thread_id.0)
    .bind(reason.as_str())
    .bind(set_by.0)
    .fetch_one(pool)
    .await?;
    row_to_block(&row)
}

/// Clear the block and append `BlockedResolved` in one tx (Cluster 386.3).
/// `None` event when the thread was not blocked (idempotent no-op).
pub async fn clear_with_event(
    pool: &PgPool,
    thread_id: ThreadId,
    resolved_by: MemberId,
) -> Result<(Option<ThreadBlock>, Option<StoredEvent>), StoreError> {
    let mut tx = pool.begin().await?;
    let row = sqlx::query(
        "DELETE FROM maidan_thread_blocks WHERE thread_id = $1
         RETURNING thread_id, reason, set_by, set_at",
    )
    .bind(thread_id.0)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(row) = row else {
        tx.commit().await?;
        return Ok((None, None));
    };
    let block = row_to_block(&row)?;
    let (workspace_id, channel_id) = events::thread_scope_in_tx(&mut tx, thread_id).await?;
    let event = Event::BlockedResolved {
        occurred_at: Utc::now(),
        workspace_id,
        channel_id,
        thread_id,
        reason: block.reason,
        resolved_by,
    };
    let stored = events::append_in_tx(&mut tx, &event).await?;
    tx.commit().await?;
    Ok((Some(block), Some(stored)))
}

pub async fn clear(pool: &PgPool, thread_id: ThreadId) -> Result<Option<ThreadBlock>, StoreError> {
    let row = sqlx::query(
        "DELETE FROM maidan_thread_blocks WHERE thread_id = $1
         RETURNING thread_id, reason, set_by, set_at",
    )
    .bind(thread_id.0)
    .fetch_optional(pool)
    .await?;
    row.as_ref().map(row_to_block).transpose()
}

pub async fn get(pool: &PgPool, thread_id: ThreadId) -> Result<Option<ThreadBlock>, StoreError> {
    let row = sqlx::query(
        "SELECT thread_id, reason, set_by, set_at
         FROM maidan_thread_blocks WHERE thread_id = $1",
    )
    .bind(thread_id.0)
    .fetch_optional(pool)
    .await?;
    row.as_ref().map(row_to_block).transpose()
}

pub async fn list_for_channel(
    pool: &PgPool,
    channel_id: ChannelId,
) -> Result<Vec<ThreadBlock>, StoreError> {
    let rows = sqlx::query(
        "SELECT b.thread_id, b.reason, b.set_by, b.set_at
         FROM maidan_thread_blocks b
         JOIN maidan_threads t ON t.id = b.thread_id
         WHERE t.channel_id = $1 AND t.tombstoned_at IS NULL
         ORDER BY b.set_at DESC, b.thread_id",
    )
    .bind(channel_id.0)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_block).collect()
}

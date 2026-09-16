use maidan_types::{
    content_hash, next_prev_hash, ChainVerifyReport, ChannelId, Event, EventLink, MessageId,
    StoredEvent, ThreadId, WorkspaceId,
};
use sqlx::{PgPool, Row};

/// Rows per batch when filling pre-Cluster-392 chain fields (Cluster 397.8).
const CHAIN_BACKFILL_BATCH: i64 = 256;

use crate::error::StoreError;
use crate::postgres::outbox;

/// Resolve a message's (workspace, channel, thread) inside a transaction
/// (Cluster 206) — see the SQLite twin.
pub async fn message_scope_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    message_id: MessageId,
) -> Result<(WorkspaceId, ChannelId, ThreadId), StoreError> {
    let row = sqlx::query(
        "SELECT c.workspace_id AS ws, t.channel_id AS ch, m.thread_id AS th
         FROM maidan_messages m
         JOIN maidan_threads t ON t.id = m.thread_id
         JOIN maidan_channels c ON c.id = t.channel_id
         WHERE m.id = $1",
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

/// Resolve a thread's (workspace, channel) inside a transaction (Cluster 208) —
/// see the SQLite twin.
pub async fn thread_scope_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    thread_id: ThreadId,
) -> Result<(WorkspaceId, ChannelId), StoreError> {
    let row = sqlx::query(
        "SELECT c.workspace_id AS ws, t.channel_id AS ch
         FROM maidan_threads t
         JOIN maidan_channels c ON c.id = t.channel_id
         WHERE t.id = $1",
    )
    .bind(thread_id.0)
    .fetch_optional(&mut **tx)
    .await?
    .ok_or(StoreError::NotFound)?;
    Ok((
        WorkspaceId(row.get::<uuid::Uuid, _>("ws")),
        ChannelId(row.get::<uuid::Uuid, _>("ch")),
    ))
}

pub async fn append(pool: &PgPool, event: &Event) -> Result<StoredEvent, StoreError> {
    let mut tx = pool.begin().await?;
    let stored = append_in_tx(&mut tx, event).await?;
    tx.commit().await?;
    Ok(stored)
}

/// Append the event + its outbox row on a caller-supplied transaction, without
/// committing (Cluster 205 transactional outbox) — see the SQLite twin.
pub async fn append_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    event: &Event,
) -> Result<StoredEvent, StoreError> {
    let mut payload = serde_json::to_value(event)?;
    // Both the hash and the stored copy must be this same normalized value, or
    // a jsonb round trip can change one without the other — see
    // `normalize_payload_numbers`.
    maidan_types::normalize_payload_numbers(&mut payload);
    let ws = event.workspace_id().map(|w| w.0);
    // Serialize chain head per workspace so two concurrent first-events cannot
    // fork genesis. `hashtextextended` is stable across backends in the cluster.
    sqlx::query(
        "SELECT pg_advisory_xact_lock(hashtextextended(COALESCE($1::text, 'maidan.event-log.unscoped'), 392))",
    )
    .bind(ws)
    .execute(&mut **tx)
    .await?;
    let previous = chain_head_in_tx(tx, ws).await?;
    let content = content_hash(&payload).map_err(|e| StoreError::InvalidInput(e.to_string()))?;
    let prev = next_prev_hash(previous.as_ref());
    // `inserted_at` is the DB insert wall-clock (Cluster 125 stability horizon),
    // distinct from the caller-supplied `occurred_at`.
    let row = sqlx::query(
        "INSERT INTO maidan_events (kind, workspace_id, channel_id, thread_id, payload, occurred_at, inserted_at, prev_hash, content_hash)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
         RETURNING id, kind, workspace_id, channel_id, thread_id, payload, occurred_at, prev_hash, content_hash",
    )
    .bind(event.kind().as_str())
    .bind(ws)
    .bind(event.channel_id().map(|c| c.0))
    .bind(event.thread_id().map(|t| t.0))
    .bind(&payload)
    .bind(event.occurred_at())
    .bind(chrono::Utc::now())
    .bind(&prev)
    .bind(&content)
    .fetch_one(&mut **tx)
    .await?;
    let stored = row_to_stored(&row)?;
    outbox::enqueue_in_tx(tx, stored.id).await?;
    Ok(stored)
}

pub async fn get_by_id(pool: &PgPool, log_id: i64) -> Result<StoredEvent, StoreError> {
    let row = sqlx::query(
        "SELECT id, kind, workspace_id, channel_id, thread_id, payload, occurred_at, prev_hash, content_hash
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
        "SELECT id, kind, workspace_id, channel_id, thread_id, payload, occurred_at, prev_hash, content_hash
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
        "SELECT id, kind, workspace_id, channel_id, thread_id, payload, occurred_at, prev_hash, content_hash
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

/// Cross-workspace events with `id > after_id`, in `id` order, capped at `limit`.
/// The bus's self-healing NOTIFY floor (Cluster 258) uses this to back-fill the
/// range missed while its `LISTEN` was disconnected — unlike [`list_after`], it is
/// not workspace-scoped, because the listener hydrates every workspace's events
/// onto the local broadcast (which then routes by workspace shard).
pub async fn list_after_global(
    pool: &PgPool,
    after_id: i64,
    limit: i64,
) -> Result<Vec<StoredEvent>, StoreError> {
    let rows = sqlx::query(
        "SELECT id, kind, workspace_id, channel_id, thread_id, payload, occurred_at, prev_hash, content_hash
         FROM maidan_events
         WHERE id > $1
         ORDER BY id ASC
         LIMIT $2",
    )
    .bind(after_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_stored).collect()
}

/// A thread's events with `id <= through_id`, in `id` order (Cluster 326) — the
/// immutable substrate for as-of context replay. The assembler folds the message
/// events into the message set as it stood at that log position.
pub async fn list_through(
    pool: &PgPool,
    thread_id: maidan_types::ThreadId,
    through_id: i64,
) -> Result<Vec<StoredEvent>, StoreError> {
    let rows = sqlx::query(
        "SELECT id, kind, workspace_id, channel_id, thread_id, payload, occurred_at, prev_hash, content_hash
         FROM maidan_events
         WHERE thread_id = $1 AND id <= $2
         ORDER BY id ASC",
    )
    .bind(thread_id.0)
    .bind(through_id)
    .fetch_all(pool)
    .await?;
    rows.iter().map(row_to_stored).collect()
}

/// Lowest retained `id` in `workspace_id` (`None` when the workspace has no
/// events). Cluster 388 uses this to fail loud on a subscribe cursor that
/// points into a pruned gap.
pub async fn min_event_id(
    pool: &PgPool,
    workspace_id: WorkspaceId,
) -> Result<Option<i64>, StoreError> {
    let row: (Option<i64>,) =
        sqlx::query_as("SELECT MIN(id) FROM maidan_events WHERE workspace_id = $1")
            .bind(workspace_id.0)
            .fetch_one(pool)
            .await?;
    Ok(row.0)
}

/// The highest event-log id (`0` when empty). The bus seeds its high-water mark
/// from this at startup so it back-fills only events appended *after* it began
/// listening, not the entire history (Cluster 258). Cluster 390 also exposes
/// this as `Store::max_event_id` for the `Maidan-Room-LSN` header (event-log
/// id, not a WAL LSN).
pub async fn max_event_id(pool: &PgPool) -> Result<i64, StoreError> {
    let row = sqlx::query("SELECT COALESCE(MAX(id), 0) AS max_id FROM maidan_events")
        .fetch_one(pool)
        .await?;
    Ok(row.get::<i64, _>("max_id"))
}

fn row_to_stored(row: &sqlx::postgres::PgRow) -> Result<StoredEvent, StoreError> {
    let kind_str: String = row.get("kind");
    let kind = parse_kind(&kind_str)?;
    let id: i64 = row.get("id");
    Ok(StoredEvent {
        id,
        lsn: id,
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
        prev_hash: row.get("prev_hash"),
        content_hash: row.get("content_hash"),
    })
}

async fn chain_head_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    workspace_id: Option<uuid::Uuid>,
) -> Result<Option<EventLink>, StoreError> {
    let row = sqlx::query(
        "SELECT id, prev_hash, content_hash FROM maidan_events
         WHERE workspace_id IS NOT DISTINCT FROM $1
         ORDER BY id DESC
         LIMIT 1",
    )
    .bind(workspace_id)
    .fetch_optional(&mut **tx)
    .await?;
    Ok(row.map(|row| {
        let id: i64 = row.get("id");
        EventLink {
            id,
            lsn: id,
            prev_hash: row.get("prev_hash"),
            content_hash: row.get("content_hash"),
        }
    }))
}

fn row_to_link(row: &sqlx::postgres::PgRow) -> EventLink {
    let id: i64 = row.get("id");
    EventLink {
        id,
        lsn: id,
        prev_hash: row.get("prev_hash"),
        content_hash: row.get("content_hash"),
    }
}

/// Oldest retained link in `workspace_id` (Cluster 393 snapshot floor).
pub async fn floor_link(
    pool: &PgPool,
    workspace_id: WorkspaceId,
) -> Result<Option<EventLink>, StoreError> {
    let row = sqlx::query(
        "SELECT id, prev_hash, content_hash FROM maidan_events
         WHERE workspace_id = $1
         ORDER BY id ASC
         LIMIT 1",
    )
    .bind(workspace_id.0)
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(row_to_link))
}

/// Newest retained link in `workspace_id` (Cluster 393 snapshot head).
pub async fn head_link(
    pool: &PgPool,
    workspace_id: WorkspaceId,
) -> Result<Option<EventLink>, StoreError> {
    let row = sqlx::query(
        "SELECT id, prev_hash, content_hash FROM maidan_events
         WHERE workspace_id = $1
         ORDER BY id DESC
         LIMIT 1",
    )
    .bind(workspace_id.0)
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(row_to_link))
}

/// Latest link in `workspace_id` with `id <= lsn` (catch-up predecessor).
pub async fn link_at_or_before(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    lsn: i64,
) -> Result<Option<EventLink>, StoreError> {
    let row = sqlx::query(
        "SELECT id, prev_hash, content_hash FROM maidan_events
         WHERE workspace_id = $1 AND id <= $2
         ORDER BY id DESC
         LIMIT 1",
    )
    .bind(workspace_id.0)
    .bind(lsn)
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(row_to_link))
}

/// Walk the workspace's retained suffix and report chain integrity.
pub async fn verify_chain(
    pool: &PgPool,
    workspace_id: WorkspaceId,
) -> Result<ChainVerifyReport, StoreError> {
    // Verify as a fold, discarding each page (Cluster 397.8). Collecting every
    // link *and* a clone of every payload first meant a large workspace was
    // gigabytes of resident memory per request — on `workspace:read`, with no
    // limit and no pagination, so repeated calls were a trivial OOM.
    const PAGE: i64 = 256;
    let mut after = 0i64;
    let mut verifier = maidan_types::ChainVerifier::new();
    loop {
        let page = list_after(pool, workspace_id, after, PAGE).await?;
        if page.is_empty() {
            return Ok(verifier.finish());
        }
        after = page.last().map(|e| e.id).unwrap_or(after);
        for stored in page {
            if let Some(report) = verifier.push(&stored.link(), &stored.payload) {
                return Ok(report);
            }
        }
    }
}

/// Fill the chain fields of rows that predate Cluster 392, in batches.
///
/// **A row that already carries a `content_hash` is never rewritten**
/// (Cluster 397.8). That is the whole security property. This used to re-link
/// *every row of every workspace* from genesis against the **current** payloads
/// whenever any single row had an empty hash — so an attacker with database
/// write access could edit a payload, blank one unrelated row's `content_hash`,
/// restart the process, and have the chain recomputed to agree with the tamper.
/// `verify_event_chain` then reported `ok: true, from_genesis: true`, which is
/// precisely the claim a tamper-evident log exists to be unable to make.
///
/// Now a blanked row is refilled from its own payload and nothing else moves, so
/// its successor's `prev_hash` — still chaining from the *original* hash — no
/// longer matches and verify breaks at that successor. Tampering is detected
/// rather than laundered.
///
/// Batched rather than one transaction per workspace: the old shape did
/// `fetch_all` of every payload in a workspace and rewrote them in a single tx,
/// which on a large deployment is an OOM and/or blows the Cluster-156 30s
/// `statement_timeout` — during migration, so the server would not boot.
pub async fn backfill_chain(pool: &PgPool) -> Result<(), StoreError> {
    loop {
        let rows = sqlx::query(
            "SELECT id, workspace_id, payload FROM maidan_events
             WHERE content_hash = ''
             ORDER BY id ASC
             LIMIT $1",
        )
        .bind(CHAIN_BACKFILL_BATCH)
        .fetch_all(pool)
        .await?;
        if rows.is_empty() {
            return Ok(());
        }
        let short = (rows.len() as i64) < CHAIN_BACKFILL_BATCH;
        for row in &rows {
            let id: i64 = row.get("id");
            let ws: Option<uuid::Uuid> = row.get("workspace_id");
            let payload: serde_json::Value = row.get("payload");
            // The predecessor inside this row's own workspace chain, whatever
            // state it is in. `None` means this row is the workspace's floor.
            let previous = previous_link(pool, ws, id).await?;
            let link = maidan_types::link_for(id, &payload, previous.as_ref())
                .map_err(|e| StoreError::InvalidInput(e.to_string()))?;
            sqlx::query(
                "UPDATE maidan_events SET prev_hash = $1, content_hash = $2
                 WHERE id = $3 AND content_hash = ''",
            )
            .bind(&link.prev_hash)
            .bind(&link.content_hash)
            .bind(id)
            .execute(pool)
            .await?;
        }
        if short {
            return Ok(());
        }
    }
}

/// The chain link of the row immediately before `id` in the same workspace, or
/// `None` when `id` is that workspace's floor.
async fn previous_link(
    pool: &PgPool,
    workspace_id: Option<uuid::Uuid>,
    id: i64,
) -> Result<Option<EventLink>, StoreError> {
    let row = sqlx::query(
        "SELECT id, prev_hash, content_hash FROM maidan_events
         WHERE workspace_id IS NOT DISTINCT FROM $1 AND id < $2
         ORDER BY id DESC
         LIMIT 1",
    )
    .bind(workspace_id)
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|row| {
        let id: i64 = row.get("id");
        EventLink {
            id,
            lsn: id,
            prev_hash: row.get("prev_hash"),
            content_hash: row.get("content_hash"),
        }
    }))
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

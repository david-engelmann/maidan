use maidan_types::{
    content_hash, next_prev_hash, ChainVerifyReport, ChannelId, ContentKeyring, Event, EventLink,
    MessageId, PeerId, StoredEvent, ThreadId, WorkspaceId,
};
use sqlx::{PgPool, Row};

/// Every column a [`StoredEvent`] is built from, with its content key joined
/// in. Pair with [`EVENTS_FROM`]; filter and order on `e.` columns.
pub(crate) const EVENT_COLUMNS: &str = "e.id, e.kind, e.workspace_id, e.channel_id, e.thread_id, e.payload, e.occurred_at, e.prev_hash, e.content_hash, e.content_key_id, k.kek_id AS key_kek_id, k.wrapped_key AS key_wrapped";
pub(crate) const EVENTS_FROM: &str =
    "maidan_events e LEFT JOIN maidan_content_keys k ON k.id = e.content_key_id";

/// Rows per batch when filling pre-chain fields.
const CHAIN_BACKFILL_BATCH: i64 = 256;

use crate::error::StoreError;
use crate::postgres::outbox;

/// Resolve a message's (workspace, channel, thread) inside a transaction — see
/// the SQLite twin.
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

/// Resolve a thread's (workspace, channel) inside a transaction — see the
/// SQLite twin.
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

pub async fn append(
    pool: &PgPool,
    keys: &ContentKeyring,
    event: &Event,
    origin: Option<PeerId>,
) -> Result<StoredEvent, StoreError> {
    let mut tx = pool.begin().await?;
    let stored = append_with_keys_in_tx(&mut tx, Some(keys), event, origin).await?;
    tx.commit().await?;
    Ok(stored)
}

/// Append the event + its outbox row on a caller-supplied transaction, without
/// committing — see the SQLite twin. Refuses an event that carries message
/// words: those go through [`append_with_keys_in_tx`].
pub async fn append_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    event: &Event,
) -> Result<StoredEvent, StoreError> {
    append_with_keys_in_tx(tx, None, event, None).await
}

/// Append, sealing a message's words under its content key before the event is
/// hashed, and shredding the key when the event withdraws the message.
/// `origin` is the federation peer the event arrived from.
pub async fn append_with_keys_in_tx(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    keys: Option<&ContentKeyring>,
    event: &Event,
    origin: Option<PeerId>,
) -> Result<StoredEvent, StoreError> {
    let mut payload = serde_json::to_value(event)?;
    crate::attribution::attach_to_payload(&mut payload)?;
    // Both the hash and the stored copy must be this same normalized value, or
    // a jsonb round trip can change one without the other — see
    // `normalize_payload_numbers`.
    maidan_types::normalize_payload_numbers(&mut payload);
    let sealing = super::content_keys::prepare_in_tx(
        tx,
        keys,
        crate::content_keys::Plan::for_event(event, origin),
        &mut payload,
    )
    .await?;
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
    // `inserted_at` is the DB insert wall-clock, distinct from the
    // caller-supplied `occurred_at`.
    let row = sqlx::query(
        "INSERT INTO maidan_events (kind, workspace_id, channel_id, thread_id, payload, occurred_at, inserted_at, prev_hash, content_hash, content_key_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
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
    .bind(sealing.content_key_id)
    .fetch_one(&mut **tx)
    .await?;
    let mut stored = row_to_stored(&row, None)?;
    stored.content_key = sealing.content_key;
    outbox::enqueue_in_tx(tx, stored.id).await?;
    Ok(stored)
}

pub async fn get_by_id(
    pool: &PgPool,
    keys: &ContentKeyring,
    log_id: i64,
) -> Result<StoredEvent, StoreError> {
    let row = sqlx::query(&format!(
        "SELECT {EVENT_COLUMNS}
         FROM {EVENTS_FROM}
         WHERE e.id = $1"
    ))
    .bind(log_id)
    .fetch_optional(pool)
    .await?;
    let Some(row) = row else {
        return Err(StoreError::NotFound);
    };
    row_to_stored(&row, Some(keys))
}

/// `keys` is `None` only for chain verification, which reads the sealed
/// payloads the hashes cover and never needs a key.
pub async fn list_after(
    pool: &PgPool,
    keys: Option<&ContentKeyring>,
    workspace_id: WorkspaceId,
    after_id: i64,
    limit: i64,
) -> Result<Vec<StoredEvent>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT {EVENT_COLUMNS}
         FROM {EVENTS_FROM}
         WHERE e.workspace_id = $1 AND e.id > $2
         ORDER BY e.id ASC
         LIMIT $3"
    ))
    .bind(workspace_id.0)
    .bind(after_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    rows.iter().map(|row| row_to_stored(row, keys)).collect()
}

/// Replay rows with `id > after_id` that are **stable** — inserted at or before
/// `stable_before` — in `id` order. Gating on `inserted_at` lets a reconcile
/// loop advance a durable cursor without stranding a lower `id` that is still
/// in flight.
pub async fn list_after_stable(
    pool: &PgPool,
    keys: &ContentKeyring,
    workspace_id: WorkspaceId,
    after_id: i64,
    stable_before: chrono::DateTime<chrono::Utc>,
    limit: i64,
) -> Result<Vec<StoredEvent>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT {EVENT_COLUMNS}
         FROM {EVENTS_FROM}
         WHERE e.workspace_id = $1 AND e.id > $2 AND e.inserted_at <= $3
         ORDER BY e.id ASC
         LIMIT $4"
    ))
    .bind(workspace_id.0)
    .bind(after_id)
    .bind(stable_before)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    rows.iter()
        .map(|row| row_to_stored(row, Some(keys)))
        .collect()
}

/// Cross-workspace events with `id > after_id`, in `id` order, capped at
/// `limit`. The bus's self-healing NOTIFY floor uses this to back-fill the
/// range missed while its `LISTEN` was disconnected — unlike [`list_after`], it
/// is not workspace-scoped, because the listener hydrates every workspace's
/// events onto the local broadcast (which then routes by workspace shard).
pub async fn list_after_global(
    pool: &PgPool,
    keys: &ContentKeyring,
    after_id: i64,
    limit: i64,
) -> Result<Vec<StoredEvent>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT {EVENT_COLUMNS}
         FROM {EVENTS_FROM}
         WHERE e.id > $1
         ORDER BY e.id ASC
         LIMIT $2"
    ))
    .bind(after_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    rows.iter()
        .map(|row| row_to_stored(row, Some(keys)))
        .collect()
}

/// [`list_after_global`], with each row's conversion result beside its id,
/// so one row that cannot be decoded (a bad kind, a content key that will not
/// unwrap) does not fail the whole page. The NOTIFY floor's back-fill uses it
/// to skip such a row and keep draining past it.
pub async fn list_after_global_each(
    pool: &PgPool,
    keys: &ContentKeyring,
    after_id: i64,
    limit: i64,
) -> Result<Vec<(i64, Result<StoredEvent, StoreError>)>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT {EVENT_COLUMNS}
         FROM {EVENTS_FROM}
         WHERE e.id > $1
         ORDER BY e.id ASC
         LIMIT $2"
    ))
    .bind(after_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|row| (row.get::<i64, _>("id"), row_to_stored(row, Some(keys))))
        .collect())
}

/// A thread's events with `id <= through_id`, in `id` order — the immutable
/// substrate for as-of context replay. The assembler folds the message events
/// into the message set as it stood at that log position.
pub async fn list_through(
    pool: &PgPool,
    keys: &ContentKeyring,
    thread_id: maidan_types::ThreadId,
    through_id: i64,
) -> Result<Vec<StoredEvent>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT {EVENT_COLUMNS}
         FROM {EVENTS_FROM}
         WHERE e.thread_id = $1 AND e.id <= $2
         ORDER BY e.id ASC"
    ))
    .bind(thread_id.0)
    .bind(through_id)
    .fetch_all(pool)
    .await?;
    rows.iter()
        .map(|row| row_to_stored(row, Some(keys)))
        .collect()
}

/// Lowest retained `id` in `workspace_id` (`None` when the workspace has no
/// events). Used to fail loud on a subscribe cursor that points into a pruned
/// gap.
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
/// listening, not the entire history. Also exposed as `Store::max_event_id` for
/// the `Maidan-Room-LSN` header (an event-log id, not a WAL LSN).
pub async fn max_event_id(pool: &PgPool) -> Result<i64, StoreError> {
    let row = sqlx::query("SELECT COALESCE(MAX(id), 0) AS max_id FROM maidan_events")
        .fetch_one(pool)
        .await?;
    Ok(row.get::<i64, _>("max_id"))
}

/// Build a [`StoredEvent`], unwrapping its joined content key when `keys` is
/// given. A row selected without the key columns (an append's `RETURNING`)
/// gets `content_key: None`.
fn row_to_stored(
    row: &sqlx::postgres::PgRow,
    keys: Option<&ContentKeyring>,
) -> Result<StoredEvent, StoreError> {
    let kind_str: String = row.get("kind");
    let kind = parse_kind(&kind_str)?;
    let id: i64 = row.get("id");
    let content_key = match keys {
        Some(keys) => crate::content_keys::unwrap_joined(
            Some(keys),
            row.get("content_key_id"),
            row.get("key_kek_id"),
            row.get("key_wrapped"),
        )?,
        None => None,
    };
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
        content_key,
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

/// Oldest retained link in `workspace_id`.
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

/// Newest retained link in `workspace_id`.
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
    // Verify as a fold, discarding each page. Collecting every link *and* a
    // clone of every payload first meant a large workspace was gigabytes of
    // resident memory per request — on `workspace:read`, with no limit and no
    // pagination, so repeated calls were a trivial OOM.
    const PAGE: i64 = 256;
    let mut after = 0i64;
    let mut verifier = maidan_types::ChainVerifier::new();
    loop {
        let page = list_after(pool, None, workspace_id, after, PAGE).await?;
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

/// Fill the chain fields of rows written before the chain existed, in batches.
///
/// **A row that already carries a `content_hash` is never rewritten**
/// That is the whole security property. This used to re-link
/// *every row of every workspace* from genesis against the **current** payloads
/// whenever any single row had an empty hash — so an attacker with database
/// write access could edit a payload, blank one unrelated row's `content_hash`,
/// restart the process, and have the chain recomputed to agree with the tamper.
/// `verify_event_chain` then reported `ok: true, from_genesis: true`, which is
/// precisely the claim a tamper-evident log exists to be unable to make.
///
/// Now a blanked row is refilled from its own payload and nothing else moves,
/// so its successor's `prev_hash` — still chaining from the *original* hash —
/// no longer matches and verify breaks at that successor. Tampering is detected
/// rather than laundered.
///
/// Batched rather than one transaction per workspace: the old shape did
/// `fetch_all` of every payload in a workspace and rewrote them in a single tx,
/// which on a large deployment is an OOM and/or blows the 30s
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
/// the single [`maidan_types::EventKind::parse`] so the wire-form mapping has
/// no per-backend copy to drift. Round-trip is guarded in `maidan-types`.
fn parse_kind(s: &str) -> Result<maidan_types::EventKind, StoreError> {
    maidan_types::EventKind::parse(s)
        .ok_or_else(|| StoreError::InvalidInput(format!("unknown event kind: {s}")))
}

/// Every workspace with at least one event — the set a chain verifier walks.
/// Derived from the log, not the workspaces table: a workspace with no events
/// has no chain, and a vacuous pass reads the same as a real one.
pub async fn workspace_ids_with_events(pool: &PgPool) -> Result<Vec<WorkspaceId>, StoreError> {
    use sqlx::Row;
    let rows = sqlx::query(
        "SELECT DISTINCT workspace_id FROM maidan_events
         WHERE workspace_id IS NOT NULL ORDER BY workspace_id",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .filter_map(|r| r.get::<Option<uuid::Uuid>, _>("workspace_id"))
        .map(WorkspaceId)
        .collect())
}

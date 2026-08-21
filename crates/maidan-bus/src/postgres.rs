//! Postgres `LISTEN`/`NOTIFY` event bus.
//!
//! In **notify** mode (default), each `publish` runs `pg_notify('maidan_events', payload)`. When
//! [`BusEnvelope::log_id`] is set (normal server path after
//! `append_event`), the payload is a small pointer and the listener
//! hydrates the full envelope from `maidan_events`. Synthetic publishes
//! with `log_id == 0` still send the full JSON envelope (tests / direct
//! bus use).
//!
//! NOTIFY payloads are capped at 7990 bytes for the legacy full-envelope
//! path only.
//!
//! In **polled** mode (`PostgresBusOptions::notify_on_publish = false`), `publish`
//! fans out on the process-local broadcast channel only (no `pg_notify`).
//! Use with outbox relay when NOTIFY is unavailable; multi-instance fan-out
//! requires notify mode or client replay.

use crate::sharded::ShardedBroadcast;
use async_trait::async_trait;
use futures::StreamExt;
use maidan_types::{BusEnvelope, Event, EventFilter};
use serde::{Deserialize, Serialize};
use sqlx::postgres::PgListener;
use sqlx::PgPool;
use tokio_stream::wrappers::{errors::BroadcastStreamRecvError, BroadcastStream};

use std::sync::Arc;

use crate::error::BusError;
use crate::hydrate_stats::{HydrateResult, HydrateStats};
use crate::item::BusItem;
use crate::listener_health::ListenerHealth;
use crate::stream::EventStream;
use crate::traits::EventBus;

const CHANNEL: &str = "maidan_events";
const PAYLOAD_LIMIT: usize = 7990;
const NOTIFY_POINTER_SCHEMA: &str = "log_id_v1";
/// Page size for the self-healing back-fill (Cluster 258): a gap or reconnect
/// drains the missed event range in batches of this size, so even a large gap
/// (a long `LISTEN` disconnect) heals without loading it all at once.
const BACKFILL_BATCH: i64 = 256;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct NotifyPointerPayload {
    notify: String,
    log_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    workspace_id: Option<uuid::Uuid>,
}

impl NotifyPointerPayload {
    fn new(log_id: i64, workspace_id: Option<maidan_types::WorkspaceId>) -> Self {
        Self {
            notify: NOTIFY_POINTER_SCHEMA.to_string(),
            log_id,
            workspace_id: workspace_id.map(|w| w.0),
        }
    }

    fn is_pointer(&self) -> bool {
        self.notify == NOTIFY_POINTER_SCHEMA && self.log_id > 0
    }
}

/// How [`PostgresBus::publish`] delivers to subscribers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PostgresBusOptions {
    /// When true (default), publish uses `pg_notify` and a LISTEN task hydrates
    /// into the local broadcast channel. When false (**polled**), publish only
    /// uses the local channel (outbox relay is the delivery path).
    pub notify_on_publish: bool,
}

impl Default for PostgresBusOptions {
    fn default() -> Self {
        Self {
            notify_on_publish: true,
        }
    }
}

#[derive(Clone)]
pub struct PostgresBus {
    pool: PgPool,
    // Cluster 201: workspace-sharded local fan-out. The LISTEN task and
    // polled-mode publishes feed this; subscribers read their workspace's shard.
    local: Arc<ShardedBroadcast>,
    notify_on_publish: bool,
    listener_health: Arc<ListenerHealth>,
    hydrate_stats: Arc<HydrateStats>,
}

impl PostgresBus {
    /// Connect with default options (NOTIFY + LISTEN).
    pub async fn connect(pool: PgPool) -> Result<Self, BusError> {
        Self::connect_with(pool, PostgresBusOptions::default()).await
    }

    /// Connect to Postgres. Starts a LISTEN fan-in task when `notify_on_publish` is true.
    pub async fn connect_with(pool: PgPool, options: PostgresBusOptions) -> Result<Self, BusError> {
        let tx = Arc::new(ShardedBroadcast::new(crate::broadcast_cap_from_env()));
        let listener_health = Arc::new(ListenerHealth::default());
        let hydrate_stats = Arc::new(HydrateStats::default());

        if options.notify_on_publish {
            let listener_tx = tx.clone();
            let listener_pool = pool.clone();
            let mut listener = PgListener::connect_with(&listener_pool).await?;
            listener.listen(CHANNEL).await?;

            let health = listener_health.clone();
            let stats = hydrate_stats.clone();
            tokio::spawn(async move {
                // High-water mark of the last event id delivered to the local
                // broadcast. Seeded from the current log head so we back-fill only
                // events appended after we started listening, not all of history.
                let mut last_seen = maidan_store::postgres::events::max_event_id(&listener_pool)
                    .await
                    .unwrap_or(0);
                loop {
                    match listener.recv().await {
                        Ok(note) => {
                            health.record_ok();
                            match decode_notify_payload(note.payload(), &stats) {
                                Ok(NotifyOutcome::Pointer(log_id)) => {
                                    // A pointer whose id sits above the high-water
                                    // plus one means we silently missed the events in
                                    // between (a lost NOTIFY across a transparent
                                    // reconnect); back-fill that middle range from the
                                    // log first, in order, so nothing is dropped.
                                    if log_id > last_seen + 1 {
                                        drain_new_events(
                                            &listener_pool,
                                            &listener_tx,
                                            last_seen,
                                            Some(log_id),
                                            &stats,
                                        )
                                        .await;
                                    }
                                    // Always hydrate the pointer's own id — never
                                    // skipping on `<= last_seen`, so a lower id that
                                    // committed late still gets delivered.
                                    match hydrate_envelope(&listener_pool, log_id).await {
                                        Ok(envelope) => {
                                            stats.record(HydrateResult::Ok);
                                            listener_tx.publish(envelope);
                                        }
                                        Err(err) => {
                                            record_hydrate_error(&stats, &err);
                                            tracing::warn!(error = %err, log_id, "drop notify pointer");
                                        }
                                    }
                                    last_seen = last_seen.max(log_id);
                                }
                                Ok(NotifyOutcome::Envelope(envelope)) => {
                                    listener_tx.publish(*envelope);
                                }
                                Err(err) => {
                                    tracing::warn!(
                                        error = %err,
                                        payload = note.payload(),
                                        "drop notify payload"
                                    );
                                }
                            }
                        }
                        Err(e) => {
                            health.record_error();
                            tracing::error!(error = %e, "pg listener errored; sleeping then retrying");
                            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                            // Reconnect catch-up: any NOTIFYs emitted while the
                            // listener was disconnected are lost, so drain the log
                            // range above the high-water to heal the gap before
                            // resuming live delivery.
                            last_seen = drain_new_events(
                                &listener_pool,
                                &listener_tx,
                                last_seen,
                                None,
                                &stats,
                            )
                            .await;
                        }
                    }
                }
            });
        }

        Ok(Self {
            pool,
            local: tx,
            notify_on_publish: options.notify_on_publish,
            listener_health,
            hydrate_stats,
        })
    }

    pub fn notify_on_publish(&self) -> bool {
        self.notify_on_publish
    }

    pub fn listener_health(&self) -> Arc<ListenerHealth> {
        self.listener_health.clone()
    }

    pub fn hydrate_stats(&self) -> Arc<HydrateStats> {
        self.hydrate_stats.clone()
    }

    /// Drain every event with `id > after_id` from the log onto the local
    /// broadcast, returning the new high-water mark (Cluster 258). The listener
    /// runs this automatically on a gap or reconnect; it is exposed so an operator
    /// (or a test) can force a heal without waiting for the next NOTIFY.
    pub async fn backfill(&self, after_id: i64) -> i64 {
        drain_new_events(&self.pool, &self.local, after_id, None, &self.hydrate_stats).await
    }
}

/// What a NOTIFY payload classifies as. A pointer defers hydration to the caller
/// (so it can distinguish the single-event fast path from a gap that needs a
/// range back-fill); a legacy full envelope carries its own event inline.
enum NotifyOutcome {
    Pointer(i64),
    Envelope(Box<BusEnvelope>),
}

fn decode_notify_payload(payload: &str, stats: &HydrateStats) -> Result<NotifyOutcome, BusError> {
    if let Ok(pointer) = serde_json::from_str::<NotifyPointerPayload>(payload) {
        if pointer.is_pointer() {
            return Ok(NotifyOutcome::Pointer(pointer.log_id));
        }
    }
    match serde_json::from_str::<BusEnvelope>(payload) {
        Ok(envelope) => Ok(NotifyOutcome::Envelope(Box::new(envelope))),
        Err(err) => {
            stats.record(HydrateResult::InvalidPayload);
            Err(err.into())
        }
    }
}

fn record_hydrate_error(stats: &HydrateStats, err: &BusError) {
    match err {
        BusError::HydrateNotFound { .. } => stats.record(HydrateResult::NotFound),
        _ => stats.record(HydrateResult::Failed),
    }
}

async fn hydrate_envelope(pool: &PgPool, log_id: i64) -> Result<BusEnvelope, BusError> {
    let stored = maidan_store::postgres::events::get_by_id(pool, log_id)
        .await
        .map_err(|err| match err {
            maidan_store::StoreError::NotFound => BusError::HydrateNotFound { log_id },
            maidan_store::StoreError::Database(e) => BusError::Database(e),
            other => BusError::HydrateFailed {
                log_id,
                reason: other.to_string(),
            },
        })?;
    envelope_from_stored(stored)
}

fn envelope_from_stored(stored: maidan_types::StoredEvent) -> Result<BusEnvelope, BusError> {
    let log_id = stored.id;
    let event: Event = serde_json::from_value(stored.payload)?;
    Ok(BusEnvelope { log_id, event })
}

/// Drain every event with `id > from_exclusive` from the log onto the local
/// broadcast, in `id` order and in bounded batches, returning the new high-water
/// mark. This is the self-healing floor (Cluster 258): it back-fills a gap the
/// live NOTIFY path missed (a coalesced/lost notification, or the range that
/// accumulated while the `LISTEN` was disconnected). Best-effort — a store error
/// stops the drain at the last id delivered, to be retried on the next NOTIFY.
async fn drain_new_events(
    pool: &PgPool,
    tx: &ShardedBroadcast,
    from_exclusive: i64,
    to_exclusive: Option<i64>,
    stats: &HydrateStats,
) -> i64 {
    let mut cursor = from_exclusive;
    loop {
        let batch = match maidan_store::postgres::events::list_after_global(
            pool,
            cursor,
            BACKFILL_BATCH,
        )
        .await
        {
            Ok(b) => b,
            Err(err) => {
                tracing::warn!(error = %err, after = cursor, "notify floor: back-fill query failed");
                return cursor;
            }
        };
        if batch.is_empty() {
            return cursor;
        }
        let len = batch.len() as i64;
        for stored in batch {
            let id = stored.id;
            // The pointer's own id (the upper bound, when set) is delivered by the
            // caller's single hydrate — back-fill only the range strictly below it.
            if to_exclusive.is_some_and(|to| id >= to) {
                return cursor;
            }
            match envelope_from_stored(stored) {
                Ok(envelope) => {
                    stats.record(HydrateResult::Backfilled);
                    tx.publish(envelope);
                }
                Err(err) => {
                    stats.record(HydrateResult::Failed);
                    tracing::warn!(error = %err, log_id = id, "notify floor: skip undecodable event");
                }
            }
            cursor = id;
        }
        if len < BACKFILL_BATCH {
            return cursor;
        }
    }
}

#[async_trait]
impl EventBus for PostgresBus {
    async fn publish(&self, envelope: BusEnvelope) -> Result<(), BusError> {
        if self.notify_on_publish {
            let payload = if envelope.log_id > 0 {
                serde_json::to_string(&NotifyPointerPayload::new(
                    envelope.log_id,
                    envelope.event.workspace_id(),
                ))?
            } else {
                let payload = serde_json::to_string(&envelope)?;
                if payload.len() > PAYLOAD_LIMIT {
                    return Err(BusError::PayloadTooLarge(payload.len()));
                }
                payload
            };
            sqlx::query("SELECT pg_notify($1, $2)")
                .bind(CHANNEL)
                .bind(&payload)
                .execute(&self.pool)
                .await?;
        } else {
            self.local.publish(envelope);
        }
        Ok(())
    }

    async fn subscribe(&self, filter: EventFilter) -> Result<EventStream, BusError> {
        let rx = self.local.subscribe(&filter);
        let stream = BroadcastStream::new(rx).filter_map(move |msg| {
            let filter = filter.clone();
            async move {
                match msg {
                    Ok(envelope) if filter.matches_envelope(&envelope) => {
                        Some(BusItem::Event(Box::new(envelope)))
                    }
                    Ok(_) => None,
                    Err(BroadcastStreamRecvError::Lagged(skipped)) => {
                        tracing::warn!(skipped, "pg bus subscriber lagged");
                        Some(BusItem::Lagged { skipped })
                    }
                }
            }
        });
        Ok(Box::pin(stream))
    }
}

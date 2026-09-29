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
use maidan_types::{BusEnvelope, ContentKeyring, EventFilter};
use serde::{Deserialize, Serialize};
use sqlx::postgres::PgListener;
use sqlx::PgPool;
use tokio_stream::wrappers::{errors::BroadcastStreamRecvError, BroadcastStream};

use std::sync::Arc;

use crate::error::BusError;
use crate::hydrate_stats::{HydrateResult, HydrateStats};
use crate::item::BusItem;
use crate::listener_health::ListenerHealth;
use crate::notify_floor::{drain, NotifyFloor, PgEventLog};
use crate::stream::EventStream;
use crate::traits::EventBus;

const CHANNEL: &str = "maidan_events";
const PAYLOAD_LIMIT: usize = 7990;
const NOTIFY_POINTER_SCHEMA: &str = "log_id_v1";

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
#[derive(Debug, Clone)]
pub struct PostgresBusOptions {
    /// When true, publish uses `pg_notify` and a LISTEN task hydrates into the
    /// local broadcast channel. When false (**polled**), publish only uses the
    /// local channel (outbox relay is the delivery path).
    pub notify_on_publish: bool,
    /// Opens the sealed message words of the events the listener hydrates;
    /// must be the store's keyring. There is no default.
    pub content_keys: Arc<ContentKeyring>,
}

impl PostgresBusOptions {
    /// NOTIFY + LISTEN delivery, opening sealed words with `content_keys`.
    pub fn new(content_keys: Arc<ContentKeyring>) -> Self {
        Self {
            notify_on_publish: true,
            content_keys,
        }
    }
}

#[derive(Clone)]
pub struct PostgresBus {
    pool: PgPool,
    keys: Arc<ContentKeyring>,
    // Workspace-sharded local fan-out. The LISTEN task and polled-mode
    // publishes feed this; subscribers read their workspace's shard.
    local: Arc<ShardedBroadcast>,
    notify_on_publish: bool,
    listener_health: Arc<ListenerHealth>,
    hydrate_stats: Arc<HydrateStats>,
}

impl PostgresBus {
    /// Connect with NOTIFY + LISTEN delivery; `content_keys` must be the
    /// store's keyring.
    pub async fn connect(
        pool: PgPool,
        content_keys: Arc<ContentKeyring>,
    ) -> Result<Self, BusError> {
        Self::connect_with(pool, PostgresBusOptions::new(content_keys)).await
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
            let log = PgEventLog {
                pool: listener_pool,
                keys: options.content_keys.clone(),
            };
            let stats = hydrate_stats.clone();
            // Seeded from the log head read after LISTEN is up, so only
            // events appended from here on are back-filled.
            let mut floor = NotifyFloor::start(log, listener_tx, stats.clone()).await?;
            tokio::spawn(async move {
                loop {
                    match listener.recv().await {
                        Ok(note) => {
                            health.record_ok();
                            match decode_notify_payload(note.payload(), &stats) {
                                Ok(NotifyOutcome::Pointer(log_id)) => {
                                    floor.on_pointer(log_id).await
                                }
                                Ok(NotifyOutcome::Envelope(envelope)) => floor.publish(*envelope),
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
                            // NOTIFYs sent while disconnected are lost: drain
                            // above the high-water before resuming.
                            floor.on_reconnect().await;
                        }
                    }
                }
            });
        }

        Ok(Self {
            pool,
            keys: options.content_keys,
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
    /// broadcast, returning the new high-water mark. The listener runs this
    /// automatically on a gap or reconnect; it is exposed so an operator (or a
    /// test) can force a heal without waiting for the next NOTIFY.
    pub async fn backfill(&self, after_id: i64) -> i64 {
        let log = PgEventLog {
            pool: self.pool.clone(),
            keys: self.keys.clone(),
        };
        drain(&log, &self.local, &self.hydrate_stats, after_id, None)
            .await
            .reached
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

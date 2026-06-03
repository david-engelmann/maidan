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

use async_trait::async_trait;
use futures::StreamExt;
use maidan_types::{BusEnvelope, Event, EventFilter};
use serde::{Deserialize, Serialize};
use sqlx::postgres::PgListener;
use sqlx::PgPool;
use tokio::sync::broadcast;
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
const BROADCAST_CAP: usize = 1024;
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
    local: broadcast::Sender<BusEnvelope>,
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
        let (tx, _) = broadcast::channel(BROADCAST_CAP);
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
                loop {
                    match listener.recv().await {
                        Ok(note) => {
                            health.record_ok();
                            match decode_notify_payload(&listener_pool, note.payload(), &stats)
                                .await
                            {
                                Ok(Some(envelope)) => {
                                    let _ = listener_tx.send(envelope);
                                }
                                Ok(None) => {}
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
}

async fn decode_notify_payload(
    pool: &PgPool,
    payload: &str,
    stats: &HydrateStats,
) -> Result<Option<BusEnvelope>, BusError> {
    if let Ok(pointer) = serde_json::from_str::<NotifyPointerPayload>(payload) {
        if pointer.is_pointer() {
            return match hydrate_envelope(pool, pointer.log_id).await {
                Ok(envelope) => {
                    stats.record(HydrateResult::Ok);
                    Ok(Some(envelope))
                }
                Err(err @ BusError::HydrateNotFound { .. }) => {
                    stats.record(HydrateResult::NotFound);
                    Err(err)
                }
                Err(err) => {
                    stats.record(HydrateResult::Failed);
                    Err(err)
                }
            };
        }
    }
    match serde_json::from_str::<BusEnvelope>(payload) {
        Ok(envelope) => Ok(Some(envelope)),
        Err(err) => {
            stats.record(HydrateResult::InvalidPayload);
            Err(err.into())
        }
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
    let event: Event = serde_json::from_value(stored.payload)?;
    Ok(BusEnvelope {
        log_id: stored.id,
        event,
    })
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
            let _ = self.local.send(envelope);
        }
        Ok(())
    }

    async fn subscribe(&self, filter: EventFilter) -> Result<EventStream, BusError> {
        let rx = self.local.subscribe();
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

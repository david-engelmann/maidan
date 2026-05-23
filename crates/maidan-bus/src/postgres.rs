//! Postgres `LISTEN`/`NOTIFY` event bus.
//!
//! Each `publish` runs `pg_notify('maidan_events', payload)` where the
//! payload is the JSON-serialized [`Event`]. Each `subscribe` spawns a
//! [`sqlx::postgres::PgListener`] task that decodes notifications,
//! applies the client-side filter, and forwards matches to the
//! subscriber's broadcast stream.
//!
//! NOTIFY payloads are capped at 7990 bytes (Postgres limit is 8000; we
//! reserve some slack). Larger payloads return [`BusError::PayloadTooLarge`].

use async_trait::async_trait;
use futures::StreamExt;
use maidan_types::{Event, EventFilter};
use sqlx::postgres::PgListener;
use sqlx::PgPool;
use tokio::sync::broadcast;
use tokio_stream::wrappers::{errors::BroadcastStreamRecvError, BroadcastStream};

use crate::error::BusError;
use crate::stream::EventStream;
use crate::traits::EventBus;

const CHANNEL: &str = "maidan_events";
const PAYLOAD_LIMIT: usize = 7990;
const BROADCAST_CAP: usize = 1024;

#[derive(Clone)]
pub struct PostgresBus {
    pool: PgPool,
    local: broadcast::Sender<Event>,
}

impl PostgresBus {
    /// Connect to Postgres and start a background listener task that
    /// fans LISTEN'd notifications into a process-local broadcast
    /// channel. The task lives as long as the returned bus does.
    pub async fn connect(pool: PgPool) -> Result<Self, BusError> {
        let (tx, _) = broadcast::channel(BROADCAST_CAP);
        let listener_tx = tx.clone();
        let listener_pool = pool.clone();

        let mut listener = PgListener::connect_with(&listener_pool).await?;
        listener.listen(CHANNEL).await?;

        tokio::spawn(async move {
            loop {
                match listener.recv().await {
                    Ok(note) => match serde_json::from_str::<Event>(note.payload()) {
                        Ok(event) => {
                            let _ = listener_tx.send(event);
                        }
                        Err(e) => {
                            tracing::warn!(error = %e, payload = note.payload(), "drop malformed event");
                        }
                    },
                    Err(e) => {
                        tracing::error!(error = %e, "pg listener errored; sleeping then retrying");
                        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
                    }
                }
            }
        });

        Ok(Self { pool, local: tx })
    }
}

#[async_trait]
impl EventBus for PostgresBus {
    async fn publish(&self, event: Event) -> Result<(), BusError> {
        let payload = serde_json::to_string(&event)?;
        if payload.len() > PAYLOAD_LIMIT {
            return Err(BusError::PayloadTooLarge(payload.len()));
        }
        sqlx::query("SELECT pg_notify($1, $2)")
            .bind(CHANNEL)
            .bind(&payload)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn subscribe(&self, filter: EventFilter) -> Result<EventStream, BusError> {
        let rx = self.local.subscribe();
        let stream = BroadcastStream::new(rx).filter_map(move |msg| {
            let filter = filter.clone();
            async move {
                match msg {
                    Ok(event) if filter.matches(&event) => Some(event),
                    Ok(_) => None,
                    Err(BroadcastStreamRecvError::Lagged(n)) => {
                        tracing::warn!(skipped = n, "pg bus subscriber lagged");
                        None
                    }
                }
            }
        });
        Ok(Box::pin(stream))
    }
}

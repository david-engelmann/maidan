//! Drains `maidan_outbox` and publishes to [`PostgresBus`] after commit.

use std::sync::Arc;
use std::time::Duration;

use maidan_bus::EventBus;
use maidan_store::postgres::{events, outbox};
use maidan_types::{BusEnvelope, Event};
use metrics::counter;
use sqlx::PgPool;
use tracing::warn;

const BATCH: i64 = 64;
const POLL_INTERVAL: Duration = Duration::from_millis(50);

pub struct OutboxRelay {
    pool: PgPool,
    bus: Arc<dyn EventBus>,
}

impl OutboxRelay {
    pub fn new(pool: PgPool, bus: Arc<dyn EventBus>) -> Self {
        Self { pool, bus }
    }

    pub async fn run(self) {
        loop {
            if let Err(err) = self.run_once().await {
                warn!(error = %err, "outbox relay tick failed");
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    }

    pub async fn run_once(&self) -> Result<(), maidan_store::StoreError> {
        let pending = outbox::list_pending(&self.pool, BATCH).await?;
        for row in pending {
            match self.relay_one(row.id, row.log_id).await {
                Ok(()) => {
                    counter!("maidan_outbox_relay_total", "result" => "ok").increment(1);
                }
                Err(err) => {
                    counter!("maidan_outbox_relay_total", "result" => "failed").increment(1);
                    let _ = outbox::record_attempt(&self.pool, row.id).await;
                    warn!(
                        outbox_id = row.id,
                        log_id = row.log_id,
                        error = %err,
                        "outbox relay failed"
                    );
                }
            }
        }
        Ok(())
    }

    async fn relay_one(&self, outbox_id: i64, log_id: i64) -> Result<(), maidan_store::StoreError> {
        let stored = events::get_by_id(&self.pool, log_id).await?;
        let event: Event = serde_json::from_value(stored.payload)?;
        let envelope = BusEnvelope {
            log_id: stored.id,
            event,
        };
        self.bus
            .publish(envelope)
            .await
            .map_err(|err| maidan_store::StoreError::InvalidInput(err.to_string()))?;
        outbox::mark_published(&self.pool, outbox_id).await?;
        Ok(())
    }
}

pub async fn pending_count(pool: &PgPool) -> Result<i64, maidan_store::StoreError> {
    outbox::count_pending(pool).await
}

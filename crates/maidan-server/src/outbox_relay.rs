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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chrono::Utc;
    use maidan_bus::test_support::FailingBus;
    use maidan_store::{postgres::events, postgres::outbox, run_postgres_migrations};
    use maidan_types::*;
    use sqlx::postgres::PgPoolOptions;
    use testcontainers::{runners::AsyncRunner, ImageExt};
    use testcontainers_modules::postgres::Postgres;

    use super::*;

    async fn postgres_pool() -> Option<(testcontainers::ContainerAsync<Postgres>, PgPool)> {
        let container = match Postgres::default()
            .with_name("pgvector/pgvector")
            .with_tag("pg17")
            .start()
            .await
        {
            Ok(c) => c,
            Err(err) => {
                eprintln!("skipping outbox_relay unit tests: docker unavailable ({err})");
                return None;
            }
        };

        let host = container.get_host().await.ok()?;
        let port = container.get_host_port_ipv4(5432).await.ok()?;
        let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
        let pool = PgPoolOptions::new()
            .max_connections(4)
            .acquire_timeout(std::time::Duration::from_secs(15))
            .connect(&url)
            .await
            .ok()?;
        run_postgres_migrations(&pool).await.ok()?;
        Some((container, pool))
    }

    #[tokio::test]
    async fn relay_failure_increments_attempts_and_leaves_row_pending() {
        let Some((_container, pool)) = postgres_pool().await else {
            return;
        };

        let event = Event::WorkspaceCreated {
            occurred_at: Utc::now(),
            workspace: Workspace {
                id: WorkspaceId(uuid::Uuid::new_v4()),
                name: "fail-ws".into(),
                created_at: Utc::now(),
                updated_at: Utc::now(),
                tombstoned_at: None,
            },
        };
        let stored = events::append(&pool, &event).await.unwrap();
        let pending = outbox::list_pending(&pool, 1).await.unwrap();
        let row = &pending[0];

        let relay = OutboxRelay::new(pool.clone(), Arc::new(FailingBus::new("injected")));
        relay.run_once().await.unwrap();

        assert_eq!(outbox::count_pending(&pool).await.unwrap(), 1);
        let after = outbox::list_pending(&pool, 1).await.unwrap();
        assert_eq!(after[0].id, row.id);
        assert_eq!(after[0].log_id, stored.id);
        assert_eq!(after[0].attempts, 1);
    }
}

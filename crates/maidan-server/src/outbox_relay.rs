//! Drains `maidan_outbox` and publishes to the configured bus after commit.

use std::sync::Arc;
use std::time::Duration;

use maidan_bus::EventBus;
use maidan_store::OutboxBackend;
use maidan_types::{BusEnvelope, Event};
use metrics::counter;
use tracing::warn;

const BATCH: i64 = 64;
const POLL_INTERVAL: Duration = Duration::from_millis(50);
pub const DEFAULT_MAX_ATTEMPTS: u32 = 16;

pub struct OutboxRelay {
    backend: OutboxBackend,
    bus: Arc<dyn EventBus>,
    max_attempts: u32,
}

impl OutboxRelay {
    pub fn new(backend: OutboxBackend, bus: Arc<dyn EventBus>) -> Self {
        Self::with_max_attempts(backend, bus, max_attempts_from_env())
    }

    pub fn with_max_attempts(
        backend: OutboxBackend,
        bus: Arc<dyn EventBus>,
        max_attempts: u32,
    ) -> Self {
        Self {
            backend,
            bus,
            max_attempts: max_attempts.max(1),
        }
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
        let pending = self.backend.list_pending(BATCH).await?;
        for row in pending {
            match self.relay_one(row.id, row.log_id).await {
                Ok(()) => {
                    counter!("maidan_outbox_relay_total", "result" => "ok").increment(1);
                }
                Err(err) => {
                    let attempts = self.backend.record_attempt(row.id).await?;
                    if attempts >= self.max_attempts as i32 {
                        let _ = self.backend.quarantine(row.id).await;
                        counter!("maidan_outbox_relay_total", "result" => "quarantined")
                            .increment(1);
                        warn!(
                            outbox_id = row.id,
                            log_id = row.log_id,
                            attempts,
                            max_attempts = self.max_attempts,
                            error = %err,
                            "outbox row quarantined after max relay attempts"
                        );
                    } else {
                        counter!("maidan_outbox_relay_total", "result" => "failed").increment(1);
                        warn!(
                            outbox_id = row.id,
                            log_id = row.log_id,
                            attempts,
                            error = %err,
                            "outbox relay failed"
                        );
                    }
                }
            }
        }
        Ok(())
    }

    async fn relay_one(&self, outbox_id: i64, log_id: i64) -> Result<(), maidan_store::StoreError> {
        let stored = self.backend.get_stored_event(log_id).await?;
        let event: Event = serde_json::from_value(stored.payload)?;
        let envelope = BusEnvelope {
            log_id: stored.id,
            event,
        };
        self.bus
            .publish(envelope)
            .await
            .map_err(|err| maidan_store::StoreError::InvalidInput(err.to_string()))?;
        self.backend.mark_published(outbox_id).await?;
        Ok(())
    }
}

pub fn max_attempts_from_env() -> u32 {
    std::env::var("MAIDAN_OUTBOX_MAX_ATTEMPTS")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|&n| n > 0)
        .unwrap_or(DEFAULT_MAX_ATTEMPTS)
}

pub async fn pending_count(backend: &OutboxBackend) -> Result<i64, maidan_store::StoreError> {
    backend.count_pending().await
}

pub async fn quarantined_count(backend: &OutboxBackend) -> Result<i64, maidan_store::StoreError> {
    backend.count_quarantined().await
}

pub async fn oldest_pending_age_secs(
    backend: &OutboxBackend,
) -> Result<Option<f64>, maidan_store::StoreError> {
    backend.oldest_relayable_pending_age_secs().await
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chrono::Utc;
    use maidan_bus::test_support::FailingBus;
    use maidan_store::{
        postgres::{events, outbox},
        run_postgres_migrations, OutboxBackend,
    };
    use maidan_types::*;
    use sqlx::postgres::PgPoolOptions;
    use testcontainers::{runners::AsyncRunner, ImageExt};
    use testcontainers_modules::postgres::Postgres;

    use super::*;

    async fn postgres_pool() -> Option<(testcontainers::ContainerAsync<Postgres>, sqlx::PgPool)> {
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
        let backend = OutboxBackend::Postgres(pool.clone());
        let pending = outbox::list_pending(&pool, 1).await.unwrap();
        let row = &pending[0];

        let relay =
            OutboxRelay::with_max_attempts(backend, Arc::new(FailingBus::new("injected")), 16);
        relay.run_once().await.unwrap();

        assert_eq!(outbox::count_pending(&pool).await.unwrap(), 1);
        let after = outbox::list_pending(&pool, 1).await.unwrap();
        assert_eq!(after[0].id, row.id);
        assert_eq!(after[0].log_id, stored.id);
        assert_eq!(after[0].attempts, 1);
    }

    #[tokio::test]
    async fn relay_quarantines_row_after_max_attempts() {
        let Some((_container, pool)) = postgres_pool().await else {
            return;
        };

        let event = Event::WorkspaceCreated {
            occurred_at: Utc::now(),
            workspace: Workspace {
                id: WorkspaceId(uuid::Uuid::new_v4()),
                name: "quarantine-ws".into(),
                created_at: Utc::now(),
                updated_at: Utc::now(),
                tombstoned_at: None,
            },
        };
        events::append(&pool, &event).await.unwrap();

        let backend = OutboxBackend::Postgres(pool.clone());
        let relay =
            OutboxRelay::with_max_attempts(backend, Arc::new(FailingBus::new("injected")), 2);
        relay.run_once().await.unwrap();
        assert_eq!(outbox::count_pending(&pool).await.unwrap(), 1);
        assert_eq!(outbox::count_quarantined(&pool).await.unwrap(), 0);

        relay.run_once().await.unwrap();
        assert_eq!(outbox::count_pending(&pool).await.unwrap(), 0);
        assert_eq!(outbox::count_quarantined(&pool).await.unwrap(), 1);
        assert!(outbox::list_pending(&pool, 8).await.unwrap().is_empty());
    }
}

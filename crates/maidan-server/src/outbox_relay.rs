//! Drains `maidan_outbox` and publishes to the configured bus after commit.

use std::sync::Arc;
use std::time::Duration;

use maidan_bus::EventBus;
use maidan_store::OutboxBackend;
use maidan_types::{BusEnvelope, Event};
use metrics::counter;
use tracing::warn;

const BATCH: i64 = 64;
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(50);
/// Idle backoff cap: an idle relay polls at most this often (Cluster 108).
const DEFAULT_MAX_POLL_INTERVAL: Duration = Duration::from_millis(1000);
pub const DEFAULT_MAX_ATTEMPTS: u32 = 16;

/// Outcome of one relay tick (Cluster 108): how many pending rows were fetched
/// and how many were successfully published. Drives the adaptive cadence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RelayTick {
    pub fetched: usize,
    pub relayed: usize,
}

/// Postgres bus + outbox relay delivery strategy (Cluster 84).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutboxRelayMode {
    /// `pg_notify` + LISTEN hydrate (default; multi-instance fan-out).
    Notify,
    /// Outbox relay publishes to the process-local bus only (no `pg_notify`).
    Polled,
}

impl OutboxRelayMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Notify => "notify",
            Self::Polled => "polled",
        }
    }
}

pub struct OutboxRelay {
    backend: OutboxBackend,
    bus: Arc<dyn EventBus>,
    max_attempts: u32,
    poll_interval: Duration,
    max_poll_interval: Duration,
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
        Self::with_options(backend, bus, max_attempts, poll_interval_from_env())
    }

    pub fn with_options(
        backend: OutboxBackend,
        bus: Arc<dyn EventBus>,
        max_attempts: u32,
        poll_interval: Duration,
    ) -> Self {
        Self {
            backend,
            bus,
            max_attempts: max_attempts.max(1),
            poll_interval,
            // Idle cap is never below the base interval.
            max_poll_interval: max_poll_interval_from_env().max(poll_interval),
        }
    }

    /// Adaptive cadence (Cluster 108): drain back-to-back while a tick fully
    /// relays a batch (more rows likely pending), then sleep when caught up —
    /// growing the idle sleep up to the cap and resetting it on any activity.
    /// Delivery semantics, metrics, and quarantine behavior are unchanged.
    pub async fn run(self) {
        let base = self.poll_interval;
        let cap = self.max_poll_interval;
        let mut idle = base;
        loop {
            match self.run_once().await {
                // Full batch, all published → backlog likely continues; drain
                // immediately with no inter-batch sleep, and reset the cadence.
                Ok(tick) if tick.relayed as i64 == BATCH => {
                    idle = base;
                    continue;
                }
                // Nothing pending → caught up; sleep and grow the idle interval.
                Ok(tick) if tick.fetched == 0 => {
                    tokio::time::sleep(idle).await;
                    idle = backoff_step(idle, base, cap);
                }
                // Partial progress (some rows failed, or < BATCH pending) →
                // sleep at the base interval and reset.
                Ok(_) => {
                    idle = base;
                    tokio::time::sleep(base).await;
                }
                Err(err) => {
                    warn!(error = %err, "outbox relay tick failed");
                    idle = base;
                    tokio::time::sleep(base).await;
                }
            }
        }
    }

    pub async fn run_once(&self) -> Result<RelayTick, maidan_store::StoreError> {
        let pending = self.backend.list_pending(BATCH).await?;
        let fetched = pending.len();
        let mut relayed = 0usize;
        for row in pending {
            match self.relay_one(row.id, row.log_id).await {
                Ok(()) => {
                    relayed += 1;
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
        Ok(RelayTick { fetched, relayed })
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

pub fn poll_interval_from_env() -> Duration {
    std::env::var("MAIDAN_OUTBOX_POLL_INTERVAL_MS")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|&ms| ms > 0)
        .map(Duration::from_millis)
        .unwrap_or(DEFAULT_POLL_INTERVAL)
}

/// Idle-backoff ceiling for the adaptive relay loop (Cluster 108).
pub fn max_poll_interval_from_env() -> Duration {
    std::env::var("MAIDAN_OUTBOX_MAX_POLL_INTERVAL_MS")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|&ms| ms > 0)
        .map(Duration::from_millis)
        .unwrap_or(DEFAULT_MAX_POLL_INTERVAL)
}

/// Double the current idle interval, clamped to `[base, cap]`. A no-op once at
/// the cap; saturates instead of overflowing.
fn backoff_step(current: Duration, base: Duration, cap: Duration) -> Duration {
    current.checked_mul(2).unwrap_or(cap).clamp(base, cap)
}

/// When false, HTTP handlers publish directly to the bus (dev/test only).
pub fn relay_enabled_from_env() -> bool {
    !matches!(
        std::env::var("MAIDAN_OUTBOX_RELAY").ok().as_deref(),
        Some("0") | Some("false") | Some("FALSE")
    )
}

pub fn relay_mode_from_env() -> OutboxRelayMode {
    match std::env::var("MAIDAN_OUTBOX_RELAY_MODE")
        .ok()
        .map(|s| s.to_ascii_lowercase())
        .as_deref()
    {
        Some("polled") => OutboxRelayMode::Polled,
        Some("notify") => OutboxRelayMode::Notify,
        Some(other) => {
            warn!(
                mode = other,
                "unknown MAIDAN_OUTBOX_RELAY_MODE; using notify"
            );
            OutboxRelayMode::Notify
        }
        None => OutboxRelayMode::Notify,
    }
}

/// Refuses disabling outbox relay in production (no silent append-then-publish downgrade).
pub fn validate_startup(production: bool, relay_enabled: bool) -> Result<(), String> {
    if production && !relay_enabled {
        return Err(
            "MAIDAN_ENV=production requires outbox relay; do not set MAIDAN_OUTBOX_RELAY=0".into(),
        );
    }
    Ok(())
}

#[cfg(test)]
mod config_tests {
    use super::*;

    #[test]
    fn production_requires_outbox_relay() {
        assert!(validate_startup(true, false).is_err());
        assert!(validate_startup(true, true).is_ok());
        assert!(validate_startup(false, false).is_ok());
    }

    #[test]
    fn idle_backoff_doubles_to_cap() {
        let base = Duration::from_millis(50);
        let cap = Duration::from_millis(1000);
        let mut idle = base;
        for expected in [100, 200, 400, 800, 1000, 1000] {
            idle = backoff_step(idle, base, cap);
            assert_eq!(idle, Duration::from_millis(expected));
        }
        // Resetting to base (the loop's action on activity) then steps again.
        assert_eq!(backoff_step(base, base, cap), Duration::from_millis(100));
    }
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
    async fn backlog_drains_in_bounded_batches_then_idles() {
        let Some((_container, pool)) = postgres_pool().await else {
            return;
        };

        // Seed two full batches plus a remainder.
        let remainder = 2usize;
        let n = 2 * BATCH as usize + remainder;
        for i in 0..n {
            let event = Event::WorkspaceCreated {
                occurred_at: Utc::now(),
                workspace: Workspace {
                    id: WorkspaceId(uuid::Uuid::new_v4()),
                    name: format!("backlog-{i}"),
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                    tombstoned_at: None,
                },
            };
            events::append(&pool, &event).await.unwrap();
        }

        let backend = OutboxBackend::Postgres(pool.clone());
        let relay =
            OutboxRelay::with_max_attempts(backend, Arc::new(maidan_bus::InMemoryBus::new()), 16);

        // Two full-batch ticks (each signals "drain immediately" via relayed == BATCH)…
        let t1 = relay.run_once().await.unwrap();
        assert_eq!(t1.relayed as i64, BATCH);
        let t2 = relay.run_once().await.unwrap();
        assert_eq!(t2.relayed as i64, BATCH);
        // …then the remainder, signalling "caught up" (< BATCH)…
        let t3 = relay.run_once().await.unwrap();
        assert_eq!(t3.fetched, remainder);
        assert_eq!(t3.relayed, remainder);
        assert_eq!(outbox::count_pending(&pool).await.unwrap(), 0);
        // …and an idle tick fetches nothing (so the loop backs off).
        let t4 = relay.run_once().await.unwrap();
        assert_eq!(t4.fetched, 0);
        assert_eq!(t4.relayed, 0);
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

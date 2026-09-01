//! Chaos / fault-injection harness (Cluster 259, Program D).
//!
//! Exercises the Cluster-258 self-healing NOTIFY floor under adversarial
//! conditions: it drives a stream of publishes at a `PostgresBus` while
//! periodically killing the `LISTEN` backend connection, then asserts that every
//! published event still reached the local broadcast — i.e. the floor back-filled
//! whatever the dropped notifications would have delivered.
//!
//! Like the Cluster-198 load harness, the end-to-end scenario is `#[ignore]`d — it
//! is a measurement / resilience tool that needs Docker and is timing-sensitive,
//! not a pass/fail CI gate. Run it explicitly:
//!
//!   cargo test -p maidan-bus --test chaos -- --ignored --nocapture
//!   MAIDAN_CHAOS_OPS=200 MAIDAN_CHAOS_KILL_EVERY=25 \
//!     cargo test -p maidan-bus --test chaos -- --ignored --nocapture
//!
//! The pure fault-schedule helper (`fault_due`) IS unit-tested in CI.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use futures::StreamExt;
use maidan_bus::{BusItem, EventBus, PostgresBus};
use maidan_store::{prelude::*, run_postgres_migrations};
use maidan_types::{BusEnvelope, Event, Workspace, WorkspaceId};
use sqlx::postgres::PgPoolOptions;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

/// Whether a fault should be injected on operation `op`, given a kill-every
/// cadence. `every == 0` disables injection; op 0 is never a fault (warm-up).
/// The pure core of the chaos soak — unit-tested in CI even though the soak
/// itself is `#[ignore]`d.
fn fault_due(op: u64, every: u64) -> bool {
    every > 0 && op > 0 && op.is_multiple_of(every)
}

fn env_u64(key: &str, default: u64) -> u64 {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

async fn postgres_pool() -> Option<(testcontainers::ContainerAsync<Postgres>, sqlx::PgPool)> {
    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            eprintln!("skipping chaos: docker unavailable ({err})");
            return None;
        }
    };
    let host = container.get_host().await.ok()?;
    let port = container.get_host_port_ipv4(5432).await.ok()?;
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .ok()?;
    run_postgres_migrations(&pool).await.ok()?;
    Some((container, pool))
}

/// Kill every backend currently running a `LISTEN` (the bus's listener
/// connection), returning how many were terminated. The listener's `recv()` then
/// errors and it reconnects — the disconnect the floor must heal.
async fn terminate_listener_backends(pool: &sqlx::PgPool) -> u64 {
    let killed: Vec<bool> = sqlx::query_scalar(
        "SELECT pg_terminate_backend(pid) FROM pg_stat_activity \
         WHERE query ILIKE 'LISTEN%' AND pid <> pg_backend_pid()",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    killed.len() as u64
}

fn workspace_event(name: &str) -> Event {
    Event::WorkspaceCreated {
        occurred_at: Utc::now(),
        workspace: Workspace {
            id: WorkspaceId(uuid::Uuid::new_v4()),
            name: name.into(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            tombstoned_at: None,
        },
    }
}

#[tokio::test]
#[ignore = "chaos soak: needs Docker, timing-sensitive — run explicitly with --ignored"]
async fn notify_floor_survives_periodic_listener_kills_under_load() {
    let Some((_container, pool)) = postgres_pool().await else {
        return;
    };
    let ops = env_u64("MAIDAN_CHAOS_OPS", 50).max(1);
    let kill_every = env_u64("MAIDAN_CHAOS_KILL_EVERY", 10);
    let delay_ms = env_u64("MAIDAN_CHAOS_DELAY_MS", 50);

    let store = PostgresStore::new(pool.clone());
    let bus = PostgresBus::connect(pool.clone()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Collect every delivered log_id in the background for the whole run.
    let received: Arc<Mutex<HashSet<i64>>> = Arc::new(Mutex::new(HashSet::new()));
    {
        let received = received.clone();
        let mut sub = bus
            .subscribe(maidan_types::EventFilter::all())
            .await
            .unwrap();
        tokio::spawn(async move {
            while let Some(item) = sub.next().await {
                if let BusItem::Event(env) = item {
                    received.lock().unwrap().insert(env.log_id);
                }
            }
        });
    }

    let mut published: HashSet<i64> = HashSet::new();
    let mut kills = 0u64;
    for op in 1..=ops {
        let event = workspace_event(&format!("chaos-{op}"));
        let stored = store.append_event(&event).await.unwrap();
        bus.publish(BusEnvelope {
            log_id: stored.id,
            event,
        })
        .await
        .unwrap();
        published.insert(stored.id);

        if fault_due(op, kill_every) {
            kills += terminate_listener_backends(&pool).await;
        }
        tokio::time::sleep(Duration::from_millis(delay_ms)).await;
    }

    // Settle: give the reconnect drain (1 s retry) + any gap back-fill time to
    // heal the tail before asserting.
    tokio::time::sleep(Duration::from_secs(4)).await;

    let got = received.lock().unwrap();
    let missing: Vec<i64> = published
        .iter()
        .copied()
        .filter(|id| !got.contains(id))
        .collect();
    eprintln!(
        "\n=== chaos report: {ops} published, {} delivered, {kills} listener kills, {} missing ===",
        got.len(),
        missing.len()
    );
    assert!(
        missing.is_empty(),
        "floor failed to heal: {} events never reached the broadcast: {missing:?}",
        missing.len()
    );
}

mod fault_tests {
    use super::fault_due;

    #[test]
    fn fault_due_fires_on_the_cadence_and_never_at_op_zero() {
        assert!(!fault_due(0, 10), "op 0 is warm-up, never a fault");
        assert!(!fault_due(5, 10));
        assert!(fault_due(10, 10));
        assert!(fault_due(20, 10));
        assert!(!fault_due(11, 10));
    }

    #[test]
    fn zero_cadence_disables_injection() {
        for op in 0..100 {
            assert!(!fault_due(op, 0), "every==0 never injects (op {op})");
        }
    }
}

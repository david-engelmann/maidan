//! Cross-replica presence over Postgres LISTEN/NOTIFY (Cluster 103.0.4).
//!
//! Two `PresenceHub`s sharing one Postgres database stand in for two server
//! replicas, each with its own `PostgresPresenceNotifier` + tasks. A member
//! online on replica A must appear in replica B's live stream and roster
//! snapshot; typing on A must reach B; and a disconnect on A must propagate an
//! offline frame to B — none of which per-process presence could do before this
//! cluster.

use std::sync::Arc;
use std::time::Duration;

use maidan_bus::{PostgresPresenceNotifier, PresenceNotifier};
use maidan_server::presence::PresenceHub;
use maidan_types::{MemberId, ThreadId, WorkspaceId};
use sqlx::postgres::PgPoolOptions;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;
use tokio::sync::broadcast;
use uuid::Uuid;

async fn pg_pool() -> Option<(testcontainers::ContainerAsync<Postgres>, sqlx::PgPool)> {
    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            eprintln!("skipping two-replica presence e2e: docker unavailable ({err})");
            return None;
        }
    };
    let host = container.get_host().await.ok()?;
    let port = container.get_host_port_ipv4(5432).await.ok()?;
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    // No migrations needed: the presence notifier only uses pg_notify / LISTEN.
    let pool = PgPoolOptions::new()
        .max_connections(8)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .ok()?;
    Some((container, pool))
}

/// Drain frames until one contains both needles, or time out.
async fn drain_until(rx: &mut broadcast::Receiver<String>, n1: &str, n2: &str) -> bool {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match rx.recv().await {
                Ok(frame) if frame.contains(n1) && frame.contains(n2) => break true,
                Ok(_) => continue,
                Err(_) => break false,
            }
        }
    })
    .await
    .unwrap_or(false)
}

#[tokio::test]
async fn presence_typing_and_disconnect_cross_replicas() {
    let Some((_container, pool)) = pg_pool().await else {
        return;
    };

    let notifier_a: Arc<dyn PresenceNotifier> = Arc::new(
        PostgresPresenceNotifier::connect(pool.clone())
            .await
            .unwrap(),
    );
    let notifier_b: Arc<dyn PresenceNotifier> = Arc::new(
        PostgresPresenceNotifier::connect(pool.clone())
            .await
            .unwrap(),
    );
    let hub_a = PresenceHub::default().with_presence_notifier(notifier_a);
    let hub_b = PresenceHub::default().with_presence_notifier(notifier_b);
    hub_a.spawn_tasks();
    hub_b.spawn_tasks();
    // Let both LISTEN tasks attach.
    tokio::time::sleep(Duration::from_millis(400)).await;

    let ws = WorkspaceId(Uuid::new_v4());
    let member_a = MemberId(Uuid::new_v4());
    let a_id = member_a.0.to_string();

    // A subscriber on replica B observes the workspace.
    let (mut rx_b, _reg_b, _snap_b) = hub_b.register(ws, MemberId(Uuid::new_v4()));

    // A member comes online on replica A.
    let (_rx_a, reg_a, _snap_a) = hub_a.register(ws, member_a);

    // ... and B's subscriber sees it (cross-replica online).
    assert!(
        drain_until(&mut rx_b, &a_id, "\"status\":\"online\"").await,
        "A's member online not delivered to B"
    );

    // A brand-new subscriber on B sees A's member in its roster snapshot.
    tokio::time::sleep(Duration::from_millis(200)).await;
    let (_rx_b2, _reg_b2, snap_b2) = hub_b.register(ws, MemberId(Uuid::new_v4()));
    assert!(
        snap_b2.contains(&a_id),
        "A's member missing from B's merged roster snapshot: {snap_b2}"
    );

    // Typing on A reaches B.
    hub_a.set_typing(ws, ThreadId(Uuid::new_v4()), member_a, true);
    assert!(
        drain_until(&mut rx_b, &a_id, "\"type\":\"typing\"").await,
        "A's typing not delivered to B"
    );

    // Disconnect on A propagates an offline frame to B.
    drop(reg_a);
    assert!(
        drain_until(&mut rx_b, &a_id, "\"status\":\"offline\"").await,
        "A's disconnect (offline) not delivered to B"
    );
}

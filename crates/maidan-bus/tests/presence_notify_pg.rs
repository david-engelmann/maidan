//! Cross-process presence fan-out over Postgres LISTEN/NOTIFY (Cluster 103).
//! Two `PostgresPresenceNotifier`s on one database stand in for two replicas:
//! an event published on one must reach a subscriber on the other.

use std::time::Duration;

use maidan_bus::{PostgresPresenceNotifier, PresenceEvent, PresenceEventKind, PresenceNotifier};
use sqlx::postgres::PgPoolOptions;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;
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
            eprintln!("skipping presence_notify_pg: docker unavailable ({err})");
            return None;
        }
    };
    let host = container.get_host().await.ok()?;
    let port = container.get_host_port_ipv4(5432).await.ok()?;
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");
    // No migrations needed: the notifier only uses pg_notify / LISTEN.
    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .ok()?;
    Some((container, pool))
}

#[tokio::test]
async fn presence_events_fan_out_across_replicas() {
    let Some((_container, pool)) = pg_pool().await else {
        return;
    };

    let replica_a = PostgresPresenceNotifier::connect(pool.clone())
        .await
        .unwrap();
    let replica_b = PostgresPresenceNotifier::connect(pool.clone())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;

    let mut sub_b = replica_b.subscribe();

    let ev = PresenceEvent {
        origin: Uuid::new_v4(),
        workspace_id: Uuid::new_v4(),
        member_id: Uuid::new_v4(),
        kind: PresenceEventKind::Typing {
            thread_id: Uuid::new_v4(),
            active: true,
        },
    };
    replica_a.publish_presence(ev.clone()).await.unwrap();

    let got = tokio::time::timeout(Duration::from_secs(5), sub_b.recv())
        .await
        .expect("timeout waiting for cross-replica presence event")
        .expect("channel closed");
    assert_eq!(got, ev);
}

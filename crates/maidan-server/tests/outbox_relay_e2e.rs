//! Postgres outbox relay delivers to the bus after commit.

use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use futures::StreamExt;
use maidan_bus::{BusItem, EventBus, PostgresBus};
use maidan_server::outbox_relay::OutboxRelay;
use maidan_store::{postgres::events, postgres::outbox, run_postgres_migrations, OutboxBackend};
use maidan_types::*;
use sqlx::postgres::PgPoolOptions;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

async fn postgres_pool() -> Option<(testcontainers::ContainerAsync<Postgres>, sqlx::PgPool)> {
    let container = match Postgres::default()
        .with_name("pgvector/pgvector")
        .with_tag("pg17")
        .start()
        .await
    {
        Ok(c) => c,
        Err(err) => {
            maidan_store::test_support::docker::skip_start_failure(err).await;
            return None;
        }
    };

    let host = container.get_host().await.expect("container host");
    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("container port");
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");

    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .expect("connect");

    run_postgres_migrations(&pool).await.expect("migrate");
    Some((container, pool))
}

#[tokio::test]
async fn relay_publishes_enqueued_event_to_bus() {
    let Some((_container, pool)) = postgres_pool().await else {
        return;
    };

    let bus = PostgresBus::connect(pool.clone(), maidan_store::test_support::dev_keys())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;

    let mut sub = bus.subscribe(EventFilter::all()).await.unwrap();

    let event = Event::WorkspaceCreated {
        occurred_at: Utc::now(),
        workspace: Workspace {
            id: WorkspaceId(uuid::Uuid::new_v4()),
            name: "relay-ws".into(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            tombstoned_at: None,
        },
    };
    let stored = events::append(&pool, &ContentKeyring::insecure_dev(), &event, None)
        .await
        .unwrap();
    assert_eq!(outbox::count_pending(&pool).await.unwrap(), 1);

    let relay = OutboxRelay::new(
        OutboxBackend::postgres(pool.clone(), maidan_store::test_support::dev_keys()),
        Arc::new(bus),
    );
    relay.run_once().await.unwrap();

    let received = tokio::time::timeout(Duration::from_secs(5), sub.next())
        .await
        .expect("timeout")
        .expect("stream ended");
    let BusItem::Event(envelope) = received else {
        panic!("expected event");
    };
    assert_eq!(envelope.log_id, stored.id);
    assert_eq!(outbox::count_pending(&pool).await.unwrap(), 0);
}

#[tokio::test]
async fn relay_run_twice_does_not_leave_duplicate_pending_rows() {
    let Some((_container, pool)) = postgres_pool().await else {
        return;
    };

    let bus = PostgresBus::connect(pool.clone(), maidan_store::test_support::dev_keys())
        .await
        .unwrap();
    let event = Event::WorkspaceCreated {
        occurred_at: Utc::now(),
        workspace: Workspace {
            id: WorkspaceId(uuid::Uuid::new_v4()),
            name: "relay-twice".into(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            tombstoned_at: None,
        },
    };
    events::append(&pool, &ContentKeyring::insecure_dev(), &event, None)
        .await
        .unwrap();
    assert_eq!(outbox::count_pending(&pool).await.unwrap(), 1);

    let relay = OutboxRelay::new(
        OutboxBackend::postgres(pool.clone(), maidan_store::test_support::dev_keys()),
        Arc::new(bus),
    );
    relay.run_once().await.unwrap();
    relay.run_once().await.unwrap();

    assert_eq!(outbox::count_pending(&pool).await.unwrap(), 0);
}

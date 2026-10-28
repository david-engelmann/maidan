//! Postgres LISTEN/NOTIFY bus integration test.

use std::time::Duration;

use chrono::Utc;
use futures::StreamExt;
use maidan_bus::{BusItem, EventBus, PostgresBus};
use maidan_store::{prelude::*, run_postgres_migrations};
use maidan_types::{BusEnvelope, *};
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
async fn round_trip_through_listen_notify_with_pointer_hydrate() {
    let Some((_container, pool)) = postgres_pool().await else {
        return;
    };

    let store = PostgresStore::for_tests(pool.clone());
    let bus = PostgresBus::connect(pool, maidan_store::test_support::dev_keys())
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(200)).await;

    let mut sub = bus.subscribe(EventFilter::all()).await.unwrap();

    let workspace = Workspace {
        id: WorkspaceId(uuid::Uuid::new_v4()),
        name: "pg-test".into(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
        tombstoned_at: None,
    };
    let event = Event::WorkspaceCreated {
        occurred_at: Utc::now(),
        workspace: workspace.clone(),
    };
    let stored = store.append_event(&event).await.unwrap();

    let hydrate_before = bus.hydrate_stats().snapshot();

    bus.publish(BusEnvelope {
        log_id: stored.id,
        event: event.clone(),
        attribution: None,
        trace: None,
    })
    .await
    .unwrap();

    let received = tokio::time::timeout(Duration::from_secs(5), sub.next())
        .await
        .expect("timeout waiting for event")
        .expect("stream closed");

    assert!(bus.listener_health().check().is_ok());
    let hydrate_after = bus.hydrate_stats().snapshot();
    assert!(hydrate_after.ok > hydrate_before.ok);

    let BusItem::Event(received) = received else {
        panic!("expected event item");
    };
    assert_eq!(received.log_id, stored.id);
    match received.event {
        Event::WorkspaceCreated { workspace: w, .. } => {
            assert_eq!(w.id, workspace.id);
            assert_eq!(w.name, "pg-test");
        }
        other => panic!("expected WorkspaceCreated, got {other:?}"),
    }
}

#[tokio::test]
async fn pointer_notify_for_missing_log_id_increments_not_found_hydrate_stat() {
    let Some((_container, pool)) = postgres_pool().await else {
        return;
    };

    let bus = PostgresBus::connect(pool.clone(), maidan_store::test_support::dev_keys())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;

    let before = bus.hydrate_stats().snapshot();
    let payload = r#"{"notify":"log_id_v1","log_id":999999999}"#;
    sqlx::query("SELECT pg_notify($1, $2)")
        .bind("maidan_events")
        .bind(payload)
        .execute(&pool)
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;

    let after = bus.hydrate_stats().snapshot();
    assert!(after.not_found > before.not_found);
}

#[tokio::test]
async fn publish_rejects_legacy_synthetic_payload_too_large() {
    let Some((_container, pool)) = postgres_pool().await else {
        return;
    };

    let bus = PostgresBus::connect(pool, maidan_store::test_support::dev_keys())
        .await
        .unwrap();

    let big_body = "x".repeat(10_000);
    let msg = Message {
        id: MessageId(uuid::Uuid::new_v4()),
        thread_id: ThreadId(uuid::Uuid::new_v4()),
        author_id: MemberId(uuid::Uuid::new_v4()),
        body: big_body,
        metadata: serde_json::json!({}),
        content: None,
        posted_at: Utc::now(),
        edited_at: None,
        tombstoned_at: None,
    };
    let event = Event::MessagePosted {
        occurred_at: Utc::now(),
        workspace_id: WorkspaceId(uuid::Uuid::new_v4()),
        channel_id: ChannelId(uuid::Uuid::new_v4()),
        thread_id: ThreadId(uuid::Uuid::new_v4()),
        dm_conversation_id: None,
        message: msg,
        sealed: None,
    };

    let err = bus
        .publish(BusEnvelope::synthetic(event))
        .await
        .unwrap_err();
    assert!(matches!(err, maidan_bus::BusError::PayloadTooLarge(_)));
}

#[tokio::test]
async fn pointer_delivery_for_large_persisted_event() {
    let Some((_container, pool)) = postgres_pool().await else {
        return;
    };

    let store = PostgresStore::for_tests(pool.clone());
    let bus = PostgresBus::connect(pool, maidan_store::test_support::dev_keys())
        .await
        .unwrap();

    tokio::time::sleep(Duration::from_millis(200)).await;

    let mut sub = bus.subscribe(EventFilter::all()).await.unwrap();

    let ws = store
        .create_workspace(maidan_types::NewWorkspace {
            name: "big-ws".into(),
        })
        .await
        .unwrap();
    let big_body = "x".repeat(10_000);
    let event = Event::MessagePosted {
        occurred_at: Utc::now(),
        workspace_id: ws.id,
        channel_id: ChannelId(uuid::Uuid::new_v4()),
        thread_id: ThreadId(uuid::Uuid::new_v4()),
        dm_conversation_id: None,
        message: Message {
            id: MessageId(uuid::Uuid::new_v4()),
            thread_id: ThreadId(uuid::Uuid::new_v4()),
            author_id: MemberId(uuid::Uuid::new_v4()),
            body: big_body.clone(),
            metadata: serde_json::json!({}),
            content: None,
            posted_at: Utc::now(),
            edited_at: None,
            tombstoned_at: None,
        },
        sealed: None,
    };
    let stored = store.append_event(&event).await.unwrap();

    bus.publish(BusEnvelope {
        log_id: stored.id,
        event,
        attribution: None,
        trace: None,
    })
    .await
    .unwrap();

    let received = tokio::time::timeout(Duration::from_secs(5), sub.next())
        .await
        .expect("timeout waiting for event")
        .expect("stream closed");

    let BusItem::Event(received) = received else {
        panic!("expected event item");
    };
    match received.event {
        Event::MessagePosted { message, .. } => assert_eq!(message.body, big_body),
        other => panic!("expected MessagePosted, got {other:?}"),
    }
}

/// The self-healing NOTIFY floor: in polled mode (no LISTEN task), events
/// appended to the log are NOT on the local broadcast until `backfill` drains
/// the missed range — which reproduces what a reconnect does after a `LISTEN`
/// disconnect drops the intervening NOTIFYs.
#[tokio::test]
async fn backfill_drains_the_missed_range_onto_the_broadcast() {
    let Some((_container, pool)) = postgres_pool().await else {
        return;
    };
    let store = PostgresStore::for_tests(pool.clone());
    // Polled mode: no listener, so nothing auto-hydrates — the gap is explicit.
    let bus = PostgresBus::connect_with(
        pool,
        maidan_bus::PostgresBusOptions {
            notify_on_publish: false,
            content_keys: store.content_keys().clone(),
        },
    )
    .await
    .unwrap();

    let mut sub = bus.subscribe(EventFilter::all()).await.unwrap();

    // Append three events straight to the log (no bus publish → no delivery).
    let mut ids = Vec::new();
    for name in ["a", "b", "c"] {
        let event = Event::WorkspaceCreated {
            occurred_at: Utc::now(),
            workspace: Workspace {
                id: WorkspaceId(uuid::Uuid::new_v4()),
                name: name.into(),
                created_at: Utc::now(),
                updated_at: Utc::now(),
                tombstoned_at: None,
            },
        };
        ids.push(store.append_event(&event).await.unwrap().id);
    }

    // Heal from a high-water below the first id: all three back-fill, in order.
    let before = bus.hydrate_stats().snapshot();
    let high_water = bus.backfill(ids[0] - 1).await;
    assert_eq!(high_water, *ids.last().unwrap(), "advances to the log head");

    for expected in &ids {
        let item = tokio::time::timeout(Duration::from_secs(5), sub.next())
            .await
            .expect("timeout")
            .expect("stream closed");
        let BusItem::Event(env) = item else {
            panic!("expected event item");
        };
        assert_eq!(env.log_id, *expected, "back-filled in id order");
    }

    let after = bus.hydrate_stats().snapshot();
    assert_eq!(
        after.backfilled - before.backfilled,
        3,
        "all three counted as backfilled"
    );

    // A second heal from the new high-water is a no-op (nothing new committed).
    assert_eq!(bus.backfill(high_water).await, high_water);
}

/// The listener's high-water mark starts at the log head read at connect,
/// so events already in the log before the bus came up are not replayed:
/// the first NOTIFY after connect delivers only its own event.
#[tokio::test]
async fn events_already_in_the_log_at_connect_are_not_replayed_by_the_first_notify() {
    let Some((_container, pool)) = postgres_pool().await else {
        return;
    };
    let store = PostgresStore::for_tests(pool.clone());
    let workspace_created = |name: &str| Event::WorkspaceCreated {
        occurred_at: Utc::now(),
        workspace: Workspace {
            id: WorkspaceId(uuid::Uuid::new_v4()),
            name: name.into(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            tombstoned_at: None,
        },
    };
    for name in ["old-a", "old-b", "old-c"] {
        store.append_event(&workspace_created(name)).await.unwrap();
    }

    let bus = PostgresBus::connect(pool, maidan_store::test_support::dev_keys())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let mut sub = bus.subscribe(EventFilter::all()).await.unwrap();

    let event = workspace_created("new");
    let stored = store.append_event(&event).await.unwrap();
    bus.publish(BusEnvelope {
        log_id: stored.id,
        event,
        attribution: None,
        trace: None,
    })
    .await
    .unwrap();

    let item = tokio::time::timeout(Duration::from_secs(5), sub.next())
        .await
        .expect("timeout waiting for event")
        .expect("stream closed");
    let BusItem::Event(first) = item else {
        panic!("expected event item");
    };
    assert_eq!(
        first.log_id, stored.id,
        "the first delivery is the new event"
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(500), sub.next())
            .await
            .is_err(),
        "nothing from before connect follows it"
    );
    assert_eq!(
        bus.hydrate_stats().snapshot().backfilled,
        0,
        "no back-fill ran"
    );
}

#[tokio::test]
async fn a_quiet_listener_recovers_its_health_after_its_connection_drops() {
    let Some((container, admin)) = postgres_pool().await else {
        return;
    };
    // The bus logs in as its own role, so the test can refuse its reconnect:
    // sqlx reconnects a dropped LISTEN transparently, and the listener only
    // errors (and reads as degraded) when that reconnect fails, as it does
    // while a hosted database is briefly unreachable.
    for statement in [
        "CREATE ROLE bus_probe LOGIN PASSWORD 'bus_probe'",
        "GRANT SELECT ON ALL TABLES IN SCHEMA public TO bus_probe",
    ] {
        sqlx::query(statement).execute(&admin).await.expect("role");
    }
    let host = container.get_host().await.expect("container host");
    let port = container
        .get_host_port_ipv4(5432)
        .await
        .expect("container port");
    let pool = PgPoolOptions::new()
        .max_connections(2)
        .acquire_timeout(Duration::from_secs(2))
        .connect(&format!(
            "postgres://bus_probe:bus_probe@{host}:{port}/postgres"
        ))
        .await
        .expect("connect as the bus role");
    let bus = PostgresBus::connect(pool, maidan_store::test_support::dev_keys())
        .await
        .expect("bus");
    let health = bus.listener_health();
    tokio::time::sleep(Duration::from_millis(200)).await;

    sqlx::query("ALTER ROLE bus_probe NOLOGIN")
        .execute(&admin)
        .await
        .expect("refuse logins");
    sqlx::query(
        "SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE usename = 'bus_probe'",
    )
    .execute(&admin)
    .await
    .expect("terminate");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    while !health.is_degraded() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the failed reconnect was noticed"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    // The database is back, and nothing is published: only the listener's
    // own probe can show that it reconnected.
    sqlx::query("ALTER ROLE bus_probe LOGIN")
        .execute(&admin)
        .await
        .expect("allow logins");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    while health.check().is_err() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "a quiet listener stayed degraded after the database came back"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

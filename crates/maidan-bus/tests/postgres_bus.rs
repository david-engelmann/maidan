//! Postgres LISTEN/NOTIFY bus integration test.

use std::time::Duration;

use chrono::Utc;
use futures::StreamExt;
use maidan_bus::{BusItem, EventBus, PostgresBus};
use maidan_store::{run_postgres_migrations, PostgresStore, Store};
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
            eprintln!("skipping postgres_bus: docker unavailable ({err})");
            return None;
        }
    };

    let host = container.get_host().await.ok()?;
    let port = container.get_host_port_ipv4(5432).await.ok()?;
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");

    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .ok()?;

    run_postgres_migrations(&pool).await.ok()?;
    Some((container, pool))
}

#[tokio::test]
async fn round_trip_through_listen_notify_with_pointer_hydrate() {
    let Some((_container, pool)) = postgres_pool().await else {
        return;
    };

    let store = PostgresStore::new(pool.clone());
    let bus = PostgresBus::connect(pool).await.unwrap();

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

    let bus = PostgresBus::connect(pool.clone()).await.unwrap();
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

    let bus = PostgresBus::connect(pool).await.unwrap();

    let big_body = "x".repeat(10_000);
    let msg = Message {
        id: MessageId(uuid::Uuid::new_v4()),
        thread_id: ThreadId(uuid::Uuid::new_v4()),
        author_id: MemberId(uuid::Uuid::new_v4()),
        body: big_body,
        metadata: serde_json::json!({}),
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

    let store = PostgresStore::new(pool.clone());
    let bus = PostgresBus::connect(pool).await.unwrap();

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
            posted_at: Utc::now(),
            edited_at: None,
            tombstoned_at: None,
        },
    };
    let stored = store.append_event(&event).await.unwrap();

    bus.publish(BusEnvelope {
        log_id: stored.id,
        event,
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

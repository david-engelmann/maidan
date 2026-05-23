//! Postgres LISTEN/NOTIFY bus integration test.

use std::time::Duration;

use chrono::Utc;
use futures::StreamExt;
use maidan_bus::{EventBus, PostgresBus};
use maidan_types::*;
use sqlx::postgres::PgPoolOptions;
use testcontainers::{runners::AsyncRunner, ImageExt};
use testcontainers_modules::postgres::Postgres;

#[tokio::test]
async fn round_trip_through_listen_notify() {
    let container = match Postgres::default().with_tag("17-alpine").start().await {
        Ok(c) => c,
        Err(err) => {
            eprintln!("skipping postgres_bus: docker unavailable ({err})");
            return;
        }
    };

    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");

    let pool = PgPoolOptions::new()
        .max_connections(4)
        .acquire_timeout(Duration::from_secs(15))
        .connect(&url)
        .await
        .unwrap();

    let bus = PostgresBus::connect(pool).await.unwrap();

    // give the background listener a tick to attach before the first publish
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

    bus.publish(event.clone()).await.unwrap();

    let received = tokio::time::timeout(Duration::from_secs(5), sub.next())
        .await
        .expect("timeout waiting for event")
        .expect("stream closed");

    match received {
        Event::WorkspaceCreated { workspace: w, .. } => {
            assert_eq!(w.id, workspace.id);
            assert_eq!(w.name, "pg-test");
        }
        other => panic!("expected WorkspaceCreated, got {other:?}"),
    }
}

#[tokio::test]
async fn publish_rejects_payload_too_large() {
    let container = match Postgres::default().with_tag("17-alpine").start().await {
        Ok(c) => c,
        Err(err) => {
            eprintln!("skipping postgres_bus: docker unavailable ({err})");
            return;
        }
    };

    let host = container.get_host().await.unwrap();
    let port = container.get_host_port_ipv4(5432).await.unwrap();
    let url = format!("postgres://postgres:postgres@{host}:{port}/postgres");

    let pool = PgPoolOptions::new()
        .max_connections(2)
        .connect(&url)
        .await
        .unwrap();
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
        message: msg,
    };

    let err = bus.publish(event).await.unwrap_err();
    assert!(matches!(err, maidan_bus::BusError::PayloadTooLarge(_)));
}

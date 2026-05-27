//! Integration tests for [`maidan_bus::test_support`] doubles.

use std::sync::Arc;

use chrono::Utc;
use maidan_bus::{test_support::RecordingBus, BusError, EventBus, InMemoryBus};
use maidan_types::*;

#[tokio::test]
async fn recording_bus_counts_publish_calls() {
    let inner = Arc::new(InMemoryBus::new());
    let bus = RecordingBus::new(inner);
    assert_eq!(bus.publishes(), 0);

    let envelope = BusEnvelope {
        log_id: 1,
        event: Event::WorkspaceCreated {
            occurred_at: Utc::now(),
            workspace: Workspace {
                id: WorkspaceId(uuid::Uuid::new_v4()),
                name: "rec".into(),
                created_at: Utc::now(),
                updated_at: Utc::now(),
                tombstoned_at: None,
            },
        },
    };
    bus.publish(envelope).await.unwrap();
    assert_eq!(bus.publishes(), 1);
}

#[tokio::test]
async fn failing_bus_returns_error_on_publish() {
    let bus = maidan_bus::test_support::FailingBus::new("nope");
    let envelope = BusEnvelope {
        log_id: 2,
        event: Event::WorkspaceCreated {
            occurred_at: Utc::now(),
            workspace: Workspace {
                id: WorkspaceId(uuid::Uuid::new_v4()),
                name: "fail".into(),
                created_at: Utc::now(),
                updated_at: Utc::now(),
                tombstoned_at: None,
            },
        },
    };
    let err = bus.publish(envelope).await.unwrap_err();
    assert!(matches!(err, BusError::HydrateFailed { .. }));
}

//! Shared fixtures for unit and integration tests.

use chrono::Utc;
use maidan_types::{EventKind, StoredEvent, WorkspaceId};

pub fn sample_stored_event(id: i64, kind: EventKind) -> StoredEvent {
    StoredEvent {
        id,
        kind,
        workspace_id: Some(WorkspaceId(uuid::Uuid::new_v4())),
        channel_id: None,
        thread_id: None,
        payload: serde_json::json!({"fixture": true}),
        occurred_at: Utc::now(),
    }
}

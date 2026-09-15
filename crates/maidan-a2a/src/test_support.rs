//! Shared fixtures for unit and integration tests.

use chrono::Utc;
use maidan_types::{link_for, EventKind, StoredEvent, WorkspaceId};

pub fn sample_stored_event(id: i64, kind: EventKind) -> StoredEvent {
    let payload = serde_json::json!({"fixture": true});
    let link = link_for(id, &payload, None).expect("fixture payload");
    StoredEvent {
        id,
        lsn: id,
        kind,
        workspace_id: Some(WorkspaceId(uuid::Uuid::new_v4())),
        channel_id: None,
        thread_id: None,
        payload,
        occurred_at: Utc::now(),
        prev_hash: link.prev_hash,
        content_hash: link.content_hash,
    }
}

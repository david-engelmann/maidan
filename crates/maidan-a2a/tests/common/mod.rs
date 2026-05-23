//! Shared fixtures for `maidan-a2a` integration tests.

use chrono::Utc;
use maidan_a2a::FederationEnvelope;
use maidan_types::{EventKind, PeerId, StoredEvent, WorkspaceId};

pub fn sample_stored_event(id: i64, kind: EventKind) -> StoredEvent {
    StoredEvent {
        id,
        kind,
        workspace_id: Some(WorkspaceId(uuid::Uuid::new_v4())),
        channel_id: None,
        thread_id: None,
        payload: serde_json::json!({"integration_fixture": true}),
        occurred_at: Utc::now(),
    }
}

pub fn sample_envelope(peer: PeerId, id: i64, kind: EventKind) -> FederationEnvelope {
    FederationEnvelope {
        origin_peer_id: peer,
        remote_event_id: id,
        event: sample_stored_event(id, kind),
    }
}

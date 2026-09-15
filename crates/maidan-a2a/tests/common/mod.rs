//! Shared fixtures for `maidan-a2a` integration tests.

use chrono::Utc;
use maidan_a2a::FederationEnvelope;
use maidan_types::{link_for, EventKind, PeerId, StoredEvent, WorkspaceId};

pub fn sample_stored_event(id: i64, kind: EventKind) -> StoredEvent {
    let payload = serde_json::json!({"integration_fixture": true});
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

pub fn sample_envelope(peer: PeerId, id: i64, kind: EventKind) -> FederationEnvelope {
    FederationEnvelope {
        origin_peer_id: peer,
        remote_event_id: id,
        event: sample_stored_event(id, kind),
    }
}

//! JSON wire-format contract tests for federation ingress bodies.

mod common;

use maidan_a2a::{FederatedEventBatch, FederationEnvelope, FederationError};
use maidan_types::{EventKind, PeerId};

#[test]
fn federation_envelope_roundtrips_json() {
    let peer = PeerId(uuid::Uuid::new_v4());
    let envelope = common::sample_envelope(peer, 99, EventKind::MessagePosted);
    let json = serde_json::to_string(&envelope).expect("serialize");
    let parsed: FederationEnvelope = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(parsed, envelope);
    parsed.validate().expect("valid after roundtrip");
}

#[test]
fn federated_event_batch_roundtrips_json() {
    let peer = PeerId(uuid::Uuid::new_v4());
    let batch = FederatedEventBatch {
        events: vec![
            common::sample_envelope(peer, 1, EventKind::WorkspaceCreated),
            common::sample_envelope(peer, 2, EventKind::MemberJoined),
        ],
    };
    let json = serde_json::to_string(&batch).expect("serialize");
    let parsed: FederatedEventBatch = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(parsed, batch);
    parsed.validate().expect("valid after roundtrip");
}

#[test]
fn envelope_rejects_unknown_json_fields() {
    let peer = uuid::Uuid::new_v4();
    let raw = format!(
        r#"{{
            "origin_peer_id": "{peer}",
            "remote_event_id": 1,
            "event": {{
                "id": 1,
                "kind": "message_posted",
                "workspace_id": "{peer}",
                "channel_id": null,
                "thread_id": null,
                "payload": {{}},
                "occurred_at": "2026-01-01T00:00:00Z"
            }},
            "unexpected": true
        }}"#
    );
    serde_json::from_str::<FederationEnvelope>(&raw).expect_err("unknown field");
}

#[test]
fn batch_validate_surfaces_nested_envelope_errors() {
    let peer = PeerId(uuid::Uuid::new_v4());
    let mut bad = common::sample_envelope(peer, 5, EventKind::VoteCast);
    bad.event.id = 4;
    let batch = FederatedEventBatch { events: vec![bad] };
    let err = batch.validate().expect_err("mismatched ids");
    assert!(matches!(err, FederationError::InvalidEnvelope(_)));
}

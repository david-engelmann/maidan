//! Property-based tests for federation envelope invariants.

mod common;

use maidan_a2a::FederationEnvelope;
use maidan_types::{EventKind, PeerId};
use proptest::prelude::*;

proptest! {
    #[test]
    fn valid_envelopes_survive_json_roundtrip(
        remote_id in 0_i64..10_000,
        peer_uuid in any::<[u8; 16]>(),
    ) {
        let peer = PeerId(uuid::Uuid::from_bytes(peer_uuid));
        let envelope = common::sample_envelope(peer, remote_id, EventKind::ThreadCreated);
        envelope.validate().expect("fixture valid");
        let json = serde_json::to_string(&envelope).expect("serialize");
        let parsed: FederationEnvelope = serde_json::from_str(&json).expect("deserialize");
        prop_assert_eq!(parsed, envelope);
    }

    #[test]
    fn mismatched_ids_always_fail_validation(
        remote_id in 1_i64..1000,
        offset in 1_i64..1000,
        peer_uuid in any::<[u8; 16]>(),
    ) {
        let peer = PeerId(uuid::Uuid::from_bytes(peer_uuid));
        let mut envelope = common::sample_envelope(peer, remote_id, EventKind::ReferenceAdded);
        envelope.event.id = remote_id.saturating_add(offset);
        prop_assume!(envelope.event.id != remote_id);
        prop_assert!(envelope.validate().is_err());
    }
}

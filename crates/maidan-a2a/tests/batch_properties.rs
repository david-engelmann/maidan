//! Property-based tests for batch validation boundaries.

mod common;

use maidan_a2a::{FederatedEventBatch, FederationError, MAX_FEDERATION_BATCH_SIZE};
use maidan_types::{EventKind, PeerId};
use proptest::prelude::*;

proptest! {
    #[test]
    fn batch_at_max_size_is_valid(peer_uuid in any::<[u8; 16]>()) {
        let peer = PeerId(uuid::Uuid::from_bytes(peer_uuid));
        let events: Vec<_> = (0..MAX_FEDERATION_BATCH_SIZE as i64)
            .map(|id| common::sample_envelope(peer, id, EventKind::ArtifactUpserted))
            .collect();
        FederatedEventBatch { events }.validate().expect("max batch ok");
    }

    #[test]
    fn batch_one_over_max_is_rejected(peer_uuid in any::<[u8; 16]>()) {
        let peer = PeerId(uuid::Uuid::from_bytes(peer_uuid));
        let events: Vec<_> = (0..=MAX_FEDERATION_BATCH_SIZE as i64)
            .map(|id| common::sample_envelope(peer, id, EventKind::MentionRecorded))
            .collect();
        let err = FederatedEventBatch { events }.validate().unwrap_err();
        if let FederationError::BatchTooLarge { count, max } = err {
            prop_assert_eq!(count, MAX_FEDERATION_BATCH_SIZE + 1);
            prop_assert_eq!(max, MAX_FEDERATION_BATCH_SIZE);
        } else {
            prop_assert!(false, "expected BatchTooLarge");
        }
    }
}

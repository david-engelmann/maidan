use serde::{Deserialize, Serialize};

use crate::envelope::FederationEnvelope;
use crate::error::FederationError;

/// Maximum events per `POST /a2a/v1/events` body (v0.6.0).
pub const MAX_FEDERATION_BATCH_SIZE: usize = 500;

/// Batch of envelopes sent on the federation ingress API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FederatedEventBatch {
    pub events: Vec<FederationEnvelope>,
}

impl FederatedEventBatch {
    pub fn validate(&self) -> Result<(), FederationError> {
        let count = self.events.len();
        if count > MAX_FEDERATION_BATCH_SIZE {
            return Err(FederationError::BatchTooLarge {
                count,
                max: MAX_FEDERATION_BATCH_SIZE,
            });
        }
        let mut seen = std::collections::HashSet::new();
        for envelope in &self.events {
            envelope.validate()?;
            let key = envelope.dedupe_key();
            if !seen.insert(key) {
                return Err(FederationError::DuplicateInBatch {
                    peer_id: key.0,
                    remote_event_id: key.1,
                });
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::envelope::FederationEnvelope;
    use crate::test_support::sample_stored_event;
    use maidan_types::{EventKind, PeerId};

    fn envelope(peer: PeerId, id: i64) -> FederationEnvelope {
        FederationEnvelope {
            origin_peer_id: peer,
            remote_event_id: id,
            event: sample_stored_event(id, EventKind::ChannelCreated),
        }
    }

    #[test]
    fn rejects_batch_larger_than_max() {
        let peer = PeerId(uuid::Uuid::new_v4());
        let events: Vec<_> = (0..MAX_FEDERATION_BATCH_SIZE + 1)
            .map(|i| envelope(peer, i as i64))
            .collect();
        let batch = FederatedEventBatch { events };
        let err = batch.validate().unwrap_err();
        assert!(matches!(err, FederationError::BatchTooLarge { .. }));
    }

    #[test]
    fn rejects_duplicate_envelopes_in_batch() {
        let peer = PeerId(uuid::Uuid::new_v4());
        let batch = FederatedEventBatch {
            events: vec![envelope(peer, 1), envelope(peer, 1)],
        };
        let err = batch.validate().unwrap_err();
        assert!(matches!(err, FederationError::DuplicateInBatch { .. }));
    }

    #[test]
    fn accepts_distinct_envelopes() {
        let peer = PeerId(uuid::Uuid::new_v4());
        let batch = FederatedEventBatch {
            events: vec![envelope(peer, 1), envelope(peer, 2)],
        };
        batch.validate().expect("valid batch");
    }
}

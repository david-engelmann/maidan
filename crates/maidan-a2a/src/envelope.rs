use maidan_types::{PeerId, StoredEvent};
use serde::{Deserialize, Serialize};

use crate::error::FederationError;

/// Wire payload for a single replicated event from a remote Maidan peer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FederationEnvelope {
    pub origin_peer_id: PeerId,
    pub remote_event_id: i64,
    pub event: StoredEvent,
}

impl FederationEnvelope {
    pub fn validate(&self) -> Result<(), FederationError> {
        if self.remote_event_id < 0 {
            return Err(FederationError::InvalidEnvelope(
                "remote_event_id must be non-negative".into(),
            ));
        }
        if self.event.id != self.remote_event_id {
            return Err(FederationError::InvalidEnvelope(
                "event.id must equal remote_event_id".into(),
            ));
        }
        if self.event.id < 0 {
            return Err(FederationError::InvalidEnvelope(
                "event.id must be non-negative".into(),
            ));
        }
        Ok(())
    }

    pub fn dedupe_key(&self) -> (PeerId, i64) {
        (self.origin_peer_id, self.remote_event_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::sample_stored_event;
    use maidan_types::EventKind;

    #[test]
    fn validate_requires_matching_event_and_remote_ids() {
        let mut event = sample_stored_event(42, EventKind::MessagePosted);
        event.id = 41;
        let envelope = FederationEnvelope {
            origin_peer_id: PeerId(uuid::Uuid::new_v4()),
            remote_event_id: 42,
            event,
        };
        let err = envelope.validate().unwrap_err();
        assert!(matches!(err, FederationError::InvalidEnvelope(_)));
    }

    #[test]
    fn validate_accepts_consistent_envelope() {
        let envelope = FederationEnvelope {
            origin_peer_id: PeerId(uuid::Uuid::new_v4()),
            remote_event_id: 7,
            event: sample_stored_event(7, EventKind::VoteCast),
        };
        envelope.validate().expect("consistent");
    }
}

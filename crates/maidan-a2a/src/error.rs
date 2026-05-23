use maidan_types::PeerId;
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum FederationError {
    #[error("invalid federation input: {0}")]
    InvalidInput(String),

    #[error("invalid envelope: {0}")]
    InvalidEnvelope(String),

    #[error("batch too large: {count} events (max {max})")]
    BatchTooLarge { count: usize, max: usize },

    #[error("duplicate envelope in batch: peer {peer_id} remote_event_id {remote_event_id}")]
    DuplicateInBatch {
        peer_id: PeerId,
        remote_event_id: i64,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_display_includes_peer_and_remote_id_for_duplicates() {
        let peer = PeerId(uuid::Uuid::new_v4());
        let err = FederationError::DuplicateInBatch {
            peer_id: peer,
            remote_event_id: 99,
        };
        let msg = err.to_string();
        assert!(msg.contains("99"));
        assert!(msg.contains(&peer.to_string()));
    }
}

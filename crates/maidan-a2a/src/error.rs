use maidan_types::PeerId;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum A2aClientError {
    #[error("http error: {0}")]
    Http(String),
    #[error("json-rpc error {code}: {message}")]
    Rpc { code: i32, message: String },
    #[error("decode error: {0}")]
    Decode(String),
    #[error("serde error: {0}")]
    Serde(#[from] serde_json::Error),
}

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

    #[error("federation transport error: {0}")]
    Transport(String),

    #[error("unauthorized")]
    Unauthorized,
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

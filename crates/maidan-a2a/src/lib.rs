//! Maidan-to-Maidan federation: types for replicating [`StoredEvent`] rows
//! across deployments. HTTP transport lands in later Cluster G PRs.

pub mod batch;
pub mod envelope;
pub mod error;
pub mod outbound;
pub mod peer;

#[cfg(test)]
pub mod test_support;

pub use batch::{FederatedEventBatch, MAX_FEDERATION_BATCH_SIZE};
pub use envelope::FederationEnvelope;
pub use error::FederationError;
pub use outbound::Outbound;
pub use peer::{validate_base_url, validate_peer_name, NewPeer, Peer};

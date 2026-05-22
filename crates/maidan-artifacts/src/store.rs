use async_trait::async_trait;
use bytes::Bytes;

use crate::error::ArtifactError;
use crate::sha::Sha256;

/// Backend-agnostic content-addressed artifact store.
///
/// `put` hashes the input and returns the sha — putting identical bytes
/// twice yields the same sha and a single physical artifact. `delete`
/// removes the body; row-level tombstones in `maidan-store` are a
/// separate concern.
#[async_trait]
pub trait ArtifactStore: Send + Sync {
    async fn put(&self, bytes: Bytes) -> Result<Sha256, ArtifactError>;
    async fn get(&self, sha: &Sha256) -> Result<Bytes, ArtifactError>;
    async fn exists(&self, sha: &Sha256) -> Result<bool, ArtifactError>;
    async fn delete(&self, sha: &Sha256) -> Result<(), ArtifactError>;
}

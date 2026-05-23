use async_trait::async_trait;
use bytes::Bytes;
use tokio::io::AsyncReadExt;

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

/// Stream bytes from `reader` into the store (buffers internally).
pub async fn put_reader(
    store: &dyn ArtifactStore,
    mut reader: impl tokio::io::AsyncRead + Unpin,
) -> Result<Sha256, ArtifactError> {
    let mut buf = Vec::new();
    reader
        .read_to_end(&mut buf)
        .await
        .map_err(ArtifactError::Io)?;
    store.put(Bytes::from(buf)).await
}

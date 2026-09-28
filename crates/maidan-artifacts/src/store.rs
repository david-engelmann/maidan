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
    fn as_any(&self) -> &dyn std::any::Any;

    async fn put(&self, bytes: Bytes) -> Result<Sha256, ArtifactError>;
    async fn get(&self, sha: &Sha256) -> Result<Bytes, ArtifactError>;
    async fn exists(&self, sha: &Sha256) -> Result<bool, ArtifactError>;
    async fn delete(&self, sha: &Sha256) -> Result<(), ArtifactError>;
}

/// Put `bytes` back if they are gone. Call it after the artifact row for `sha`
/// has committed: a last-reference erase may have reaped the bytes between
/// this upload's `put` and its row. Once the row exists nothing reaps them, so
/// after this returns the bytes are there for as long as the row is.
pub async fn restore_if_reaped(
    store: &dyn ArtifactStore,
    sha: &Sha256,
    bytes: Bytes,
) -> Result<(), ArtifactError> {
    if store.exists(sha).await? {
        return Ok(());
    }
    let restored = store.put(bytes).await?;
    if &restored != sha {
        return Err(ArtifactError::InvalidInput(format!(
            "restored bytes hash to {restored}, not {sha}"
        )));
    }
    Ok(())
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

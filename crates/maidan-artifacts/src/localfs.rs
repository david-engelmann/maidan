//! Filesystem implementation of [`ArtifactStore`].
//!
//! Files live under `<root>/<sha[0:2]>/<sha[2:4]>/<sha[4:]>` so a single
//! directory never holds more than ~256 entries at any fan-out level.
//! Writes are atomic: bytes are written to a per-task tempfile and
//! renamed into place. Concurrent puts of identical content collapse to
//! one final file because rename is atomic on the same filesystem.

use std::path::{Path, PathBuf};

use async_trait::async_trait;
use bytes::Bytes;
use tokio::fs;

use crate::error::ArtifactError;
use crate::path::object_key;
use crate::sha::Sha256;
use crate::store::ArtifactStore;

#[derive(Debug, Clone)]
pub struct LocalFsStore {
    root: PathBuf,
}

impl LocalFsStore {
    /// Create a store rooted at `root`. The directory is created on first
    /// `put`; callers do not need to pre-create it.
    pub fn new<P: Into<PathBuf>>(root: P) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn body_path(&self, sha: &Sha256) -> PathBuf {
        self.root.join(object_key(sha))
    }

    fn tmp_path(&self) -> PathBuf {
        self.root.join(format!(".tmp-{}", uuid::Uuid::new_v4()))
    }
}

#[async_trait]
impl ArtifactStore for LocalFsStore {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    async fn put(&self, bytes: Bytes) -> Result<Sha256, ArtifactError> {
        let sha = Sha256::compute(&bytes);
        let final_path = self.body_path(&sha);

        if fs::try_exists(&final_path).await? {
            return Ok(sha);
        }

        fs::create_dir_all(&self.root).await?;
        if let Some(parent) = final_path.parent() {
            fs::create_dir_all(parent).await?;
        }

        let tmp = self.tmp_path();
        // Write to tempfile; drop on any error path so we don't leak.
        if let Err(e) = fs::write(&tmp, &bytes).await {
            let _ = fs::remove_file(&tmp).await;
            return Err(ArtifactError::Io(e));
        }

        // rename() is atomic on the same filesystem. If a concurrent put
        // landed the file first, rename will either succeed (we replace
        // identical content harmlessly) or fail on platforms that disallow
        // overwrite; in either case the final result is one file with the
        // correct content.
        match fs::rename(&tmp, &final_path).await {
            Ok(()) => Ok(sha),
            Err(e) => {
                let _ = fs::remove_file(&tmp).await;
                if fs::try_exists(&final_path).await? {
                    Ok(sha)
                } else {
                    Err(ArtifactError::Io(e))
                }
            }
        }
    }

    async fn get(&self, sha: &Sha256) -> Result<Bytes, ArtifactError> {
        match fs::read(self.body_path(sha)).await {
            Ok(b) => Ok(Bytes::from(b)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(ArtifactError::NotFound),
            Err(e) => Err(ArtifactError::Io(e)),
        }
    }

    async fn exists(&self, sha: &Sha256) -> Result<bool, ArtifactError> {
        Ok(fs::try_exists(self.body_path(sha)).await?)
    }

    async fn delete(&self, sha: &Sha256) -> Result<(), ArtifactError> {
        match fs::remove_file(self.body_path(sha)).await {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Err(ArtifactError::NotFound),
            Err(e) => Err(ArtifactError::Io(e)),
        }
    }
}

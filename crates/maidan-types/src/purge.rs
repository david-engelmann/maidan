//! Workspace erasure audit types.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::WorkspaceId;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WorkspacePurgeResult {
    pub workspace_id: WorkspaceId,
    pub messages_tombstoned: u64,
    pub messages_purged: u64,
    /// Embedding rows removed.
    pub embeddings_removed: u64,
    pub references_removed: u64,
    pub api_tokens_revoked: u64,
    pub events_removed: u64,
    /// Message content keys destroyed with the events (crypto-shredding).
    pub content_keys_destroyed: u64,
    /// Artifact metadata rows removed; SHA-256 hex for blob purge.
    pub artifacts_removed: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub artifact_shas: Vec<String>,
    pub occurred_at: DateTime<Utc>,
}

/// One workspace's reference to a shared artifact, erased. Artifacts are
/// content-addressed and deduplicated across workspaces, so the bytes go only
/// with the last reference.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ArtifactErasure {
    pub workspace_id: WorkspaceId,
    pub sha256: String,
    /// No workspace references the artifact any more: its row is gone and its
    /// bytes are deleted from blob storage.
    pub last_reference: bool,
    pub occurred_at: DateTime<Utc>,
}

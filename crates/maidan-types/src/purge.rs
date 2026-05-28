//! Workspace erasure audit types (Cluster 25).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::WorkspaceId;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WorkspacePurgeResult {
    pub workspace_id: WorkspaceId,
    pub messages_tombstoned: u64,
    pub messages_purged: u64,
    /// Embedding rows removed (Cluster 28; CASCADE on message delete).
    pub embeddings_removed: u64,
    pub references_removed: u64,
    pub api_tokens_revoked: u64,
    pub events_removed: u64,
    pub occurred_at: DateTime<Utc>,
}

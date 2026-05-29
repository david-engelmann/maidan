//! Workspace full erasure result (Cluster 53).

use serde::{Deserialize, Serialize};

use crate::purge::WorkspacePurgeResult;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WorkspaceEraseResult {
    pub purge: WorkspacePurgeResult,
    pub workspace_erased: bool,
}

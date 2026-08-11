//! Per-workspace usage snapshot for metering / quota visibility (Cluster 188).

use serde::{Deserialize, Serialize};

use crate::ids::WorkspaceId;

/// Live counts for one workspace, computed on demand. A metering/billing basis
/// that stays low-cardinality (a per-request DB aggregate, not a scraped
/// per-tenant Prometheus series). Counts exclude tombstoned rows.
///
/// Artifact storage is intentionally omitted: blobs are content-addressed and
/// deduplicated **across** workspaces, so per-tenant bytes is ill-defined
/// (tracked in Open Work).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WorkspaceUsage {
    pub workspace_id: WorkspaceId,
    pub members: i64,
    pub channels: i64,
    pub threads: i64,
    pub messages: i64,
}

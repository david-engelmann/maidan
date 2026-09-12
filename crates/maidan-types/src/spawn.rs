//! Spawn budget (Cluster 376, Wave 2 #23, G6/G-dev-3/W3).
//!
//! A per-workspace cap on how far an agent family may fan out: `max_children`
//! (direct child threads per parent), `max_depth` (thread nesting), and
//! `max_tools` (tool calls recorded on a thread). Coordination cost grows as
//! n(n-1)/2 (Brooks/Amdahl/Two-Pizza), so a runaway that keeps spawning helpers
//! is refused past the cap — a `SpawnRejected` error + a `ThreadSpawnDenied`
//! event. Each axis is opt-in: a `None` (absent row or NULL column) is unlimited
//! on that axis, like the WIP limit (Cluster 362).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::WorkspaceId;

/// A workspace's spawn budget. Each limit is `None` = unlimited on that axis.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SpawnBudget {
    pub workspace_id: WorkspaceId,
    /// Max direct child threads per parent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_children: Option<i64>,
    /// Max thread nesting depth (a root thread is depth 1).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_depth: Option<i64>,
    /// Max tool calls recorded on a thread (across its messages' tool-use blocks).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tools: Option<i64>,
    pub updated_at: DateTime<Utc>,
}

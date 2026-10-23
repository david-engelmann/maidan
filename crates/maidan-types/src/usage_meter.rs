//! Per-workspace usage meters for the hosted service's free allowance: what
//! each workspace used, per meter, per UTC day. Nothing is cut off or billed
//! on them; they say what a workspace costs.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::WorkspaceId;

/// What is metered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum UsageMeter {
    /// Authenticated HTTP requests.
    Requests,
    /// MCP tool calls.
    McpCalls,
    /// Texts embedded: messages indexed and search queries.
    EmbeddingCalls,
    /// Outbound deliveries sent: Slack, GitHub and Linear egress, email and
    /// web push.
    Deliveries,
    /// Milliseconds of WASI handler execution.
    WasiMs,
    /// Bytes stored for the workspace: its artifacts and its message bodies.
    /// A gauge: the day's row is the latest measurement, not a sum.
    StoredBytes,
}

impl UsageMeter {
    pub const ALL: [UsageMeter; 6] = [
        UsageMeter::Requests,
        UsageMeter::McpCalls,
        UsageMeter::EmbeddingCalls,
        UsageMeter::Deliveries,
        UsageMeter::WasiMs,
        UsageMeter::StoredBytes,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Requests => "requests",
            Self::McpCalls => "mcp_calls",
            Self::EmbeddingCalls => "embedding_calls",
            Self::Deliveries => "deliveries",
            Self::WasiMs => "wasi_ms",
            Self::StoredBytes => "stored_bytes",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|meter| meter.as_str() == s)
    }
}

/// One meter's total for one workspace on one UTC day.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UsageMeterDay {
    pub workspace_id: WorkspaceId,
    pub day: NaiveDate,
    pub meter: UsageMeter,
    pub quantity: i64,
}

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
}

impl UsageMeter {
    pub const ALL: [UsageMeter; 2] = [UsageMeter::Requests, UsageMeter::McpCalls];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Requests => "requests",
            Self::McpCalls => "mcp_calls",
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

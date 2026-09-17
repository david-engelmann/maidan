//! Spawn budget.
//!
//! A per-workspace cap on how far an agent family may fan out: `max_children`
//! (direct child threads per parent), `max_depth` (thread nesting), and
//! `max_tools` (tool calls recorded on a thread). Coordination cost grows as
//! n(n-1)/2 (Brooks/Amdahl/Two-Pizza), so a runaway that keeps spawning helpers
//! is refused past the cap — a `SpawnRejected` error + a `ThreadSpawnDenied`
//! event. Each axis is opt-in: a `None` (absent row or NULL column) is
//! unlimited on that axis, like the WIP limit.

use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::events::Event;
use crate::ids::{ChannelId, MemberId, ThreadId, WorkspaceId};

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

/// Which spawn-budget axis refused a spawn — a small controlled vocabulary,
/// carried on the `ThreadSpawnDenied` event in its `as_str` form (like
/// [`BudgetReason`] on `ClaimFailed`).
///
/// [`BudgetReason`]: crate::models::BudgetReason
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpawnAxis {
    Children,
    Depth,
    Tools,
}

impl SpawnAxis {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Children => "children",
            Self::Depth => "depth",
            Self::Tools => "tools",
        }
    }
}

/// A spawn the budget refused — the scope, the axis, the numbers. Carried by
/// `StoreError::SpawnRejected` so one refusal serves both purposes: its
/// `Display` is the client-facing message (unchanged from 376.2/376.3), and its
/// fields are the `ThreadSpawnDenied` payload. The **actor** is deliberately
/// not part of it — the store's thread-create path has no author, so the route
/// supplies who tried.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnDenial {
    pub workspace_id: WorkspaceId,
    pub channel_id: ChannelId,
    /// The parent thread whose fan-out was capped (`children`/`depth`), or the
    /// thread whose tool calls were capped (`tools`).
    pub thread_id: ThreadId,
    pub axis: SpawnAxis,
    /// The configured cap on that axis.
    pub limit: i64,
    /// What the thread already holds on that axis.
    pub observed: i64,
}

impl SpawnDenial {
    /// The `ThreadSpawnDenied` observability event for this refusal. `actor` is
    /// who tried to spawn — the one fact the store's gate can't know on the
    /// thread-create path — so REST and MCP build the same event from the same
    /// denial. `None` is an unattributed caller (bypass auth), matching how the
    /// audit trail records its `actor_id`.
    pub fn denied_event(&self, actor: Option<MemberId>) -> Event {
        Event::ThreadSpawnDenied {
            occurred_at: Utc::now(),
            workspace_id: self.workspace_id,
            channel_id: self.channel_id,
            thread_id: self.thread_id,
            member_id: actor,
            axis: self.axis.as_str().to_string(),
            limit: self.limit,
            observed: self.observed,
        }
    }
}

impl fmt::Display for SpawnDenial {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.axis {
            SpawnAxis::Children => write!(
                f,
                "spawn budget: the parent thread already has the maximum {} child threads",
                self.limit
            ),
            SpawnAxis::Depth => {
                write!(f, "spawn budget: max nesting depth {} reached", self.limit)
            }
            SpawnAxis::Tools => write!(
                f,
                "spawn budget: max {} tool calls per thread reached ({} already recorded)",
                self.limit, self.observed
            ),
        }
    }
}

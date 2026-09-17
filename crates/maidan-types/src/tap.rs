//! Tap projector contract.
//!
//! A tap is any consumer of the event log that is **not** the log: webhook
//! delivery, WebSocket / MCP-SSE subscribe, AG-UI, and search. Search is a
//! projector. It must not diverge from the log silently.
//!
//! Contract, fail closed:
//!
//! 1. **verify** — every backfill page is hash-chain checked
//!    ([`crate::verify_catch_up`]). A break is 409 / rebuild, not a skip.
//! 2. **backfill** — drain the log (or a snapshot + catch-up) before live.
//! 3. **filter** — [`crate::ProjectorShape`] (`workspace`, optional
//!    channel/thread, `types[]`). Unknown types fail loud.
//! 4. **live-waits-for-history** — do not emit live frames until the
//!    history high-water is at the workspace (or shape) head observed
//!    at subscribe. Compare to that head, **not** the global Room-LSN
//!    (other tenants move the global watermark).
//! 5. **webhook / WS** — same rules as SSE. Those paths already do
//!    HTTP-then-WS and `Lagged`→log resume; this module names the
//!    contract they must keep.
//!
//! Missing or broken history: [`crate::CursorTooOld`] (refetch
//! [`crate::LogSnapshot`]) or a chain-break report. Never clamp.

use serde::{Deserialize, Serialize};

use crate::cursor::ProjectorShape;
use crate::event_chain::ChainBreakReason;
use crate::events::EventKind;
use crate::ids::WorkspaceId;
use crate::log_snapshot::LogSnapshot;

/// Surfaces bound by the tap contract.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum TapSurface {
    Webhook,
    Websocket,
    McpSse,
    Search,
    AgUi,
}

impl TapSurface {
    pub const ALL: &[Self] = &[
        Self::Webhook,
        Self::Websocket,
        Self::McpSse,
        Self::Search,
        Self::AgUi,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Webhook => "webhook",
            Self::Websocket => "websocket",
            Self::McpSse => "mcp_sse",
            Self::Search => "search",
            Self::AgUi => "ag_ui",
        }
    }

    /// Event kinds this surface projects. `None` = the caller's
    /// [`ProjectorShape`] (empty types = all kinds).
    pub fn default_kinds(self) -> Option<&'static [EventKind]> {
        match self {
            Self::Search => Some(SEARCH_PROJECTOR_KINDS),
            Self::Webhook | Self::Websocket | Self::McpSse | Self::AgUi => None,
        }
    }
}

/// Message events the search indexer must project. Anything else is
/// out of the lexical/embedding index on purpose.
pub const SEARCH_PROJECTOR_KINDS: &[EventKind] = &[
    EventKind::MessagePosted,
    EventKind::MessageEdited,
    EventKind::MessageTombstoned,
];

/// Named rules a tap implements. Serialisable so REST/MCP can show the
/// contract; enforcement lives at each surface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TapContract {
    pub verify: bool,
    pub backfill: bool,
    pub live_waits_for_history: bool,
    pub filter: ProjectorShape,
    pub surface: TapSurface,
}

impl TapContract {
    pub fn for_surface(surface: TapSurface, workspace_id: WorkspaceId) -> Self {
        let mut filter = ProjectorShape::workspace(workspace_id);
        if let Some(kinds) = surface.default_kinds() {
            filter.types = kinds.to_vec();
        }
        Self {
            verify: true,
            backfill: true,
            live_waits_for_history: true,
            filter,
            surface,
        }
    }

    pub fn is_strict(&self) -> bool {
        self.verify && self.backfill && self.live_waits_for_history
    }
}

/// Fail-closed reasons a tap must not swallow.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(tag = "fault", rename_all = "snake_case")]
pub enum TapFault {
    CursorTooOld {
        after_id: i64,
        oldest_id: i64,
        snapshot: String,
    },
    ChainBreak {
        break_at: Option<i64>,
        reason: ChainBreakReason,
    },
    LiveBeforeHistory {
        history_hw: i64,
        head_lsn: i64,
    },
    MissingHistory,
}

impl TapFault {
    pub fn cursor_too_old(after_id: i64, oldest_id: i64, workspace_id: WorkspaceId) -> Self {
        Self::CursorTooOld {
            after_id,
            oldest_id,
            snapshot: LogSnapshot::path(workspace_id),
        }
    }

    /// Search must rebuild (reindex from the log / snapshot) rather than
    /// serve a silently gapped index.
    pub fn search_must_rebuild(&self) -> bool {
        matches!(
            self,
            Self::CursorTooOld { .. }
                | Self::ChainBreak { .. }
                | Self::MissingHistory
                | Self::LiveBeforeHistory { .. }
        )
    }
}

/// History has caught up to the workspace (or shape) head. Live may start.
///
/// `head_lsn` is **not** the global Room-LSN. A workspace-scoped tap
/// that waited on the global watermark would stall on other tenants.
pub fn history_caught_up(history_high_water: i64, head_lsn: i64) -> bool {
    history_high_water >= head_lsn
}

/// Live delivery before history is a contract break (fail closed).
pub fn live_before_history(history_high_water: i64, head_lsn: i64) -> Option<TapFault> {
    if history_caught_up(history_high_water, head_lsn) {
        None
    } else {
        Some(TapFault::LiveBeforeHistory {
            history_hw: history_high_water,
            head_lsn,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::WorkspaceId;
    use uuid::Uuid;

    fn ws() -> WorkspaceId {
        WorkspaceId(Uuid::from_u128(7))
    }

    #[test]
    fn search_contract_is_strict_and_message_filtered() {
        let tap = TapContract::for_surface(TapSurface::Search, ws());
        assert!(tap.is_strict());
        assert_eq!(tap.filter.types, SEARCH_PROJECTOR_KINDS);
        assert_eq!(
            TapSurface::Search.default_kinds(),
            Some(SEARCH_PROJECTOR_KINDS)
        );
        assert!(TapSurface::Websocket.default_kinds().is_none());
    }

    #[test]
    fn webhook_and_ws_share_the_strict_contract() {
        for surface in [
            TapSurface::Webhook,
            TapSurface::Websocket,
            TapSurface::McpSse,
            TapSurface::AgUi,
        ] {
            let tap = TapContract::for_surface(surface, ws());
            assert!(tap.verify);
            assert!(tap.backfill);
            assert!(tap.live_waits_for_history);
            assert!(tap.filter.types.is_empty());
        }
        assert_eq!(TapSurface::ALL.len(), 5);
    }

    #[test]
    fn live_waits_for_workspace_head_not_global_room() {
        assert!(history_caught_up(10, 10));
        assert!(history_caught_up(11, 10));
        assert!(!history_caught_up(9, 10));
        assert!(history_caught_up(0, 0));
        assert!(live_before_history(9, 10).is_some());
        assert!(live_before_history(10, 10).is_none());
    }

    #[test]
    fn missing_history_and_chain_break_force_search_rebuild() {
        let too_old = TapFault::cursor_too_old(50, 100, ws());
        match &too_old {
            TapFault::CursorTooOld { snapshot, .. } => {
                assert_eq!(snapshot, &LogSnapshot::path(ws()));
            }
            _ => panic!("expected cursor_too_old"),
        }
        assert!(too_old.search_must_rebuild());
        assert!(TapFault::MissingHistory.search_must_rebuild());
        assert!(TapFault::ChainBreak {
            break_at: Some(3),
            reason: ChainBreakReason::ContentHashMismatch,
        }
        .search_must_rebuild());
        assert!(TapFault::LiveBeforeHistory {
            history_hw: 1,
            head_lsn: 9,
        }
        .search_must_rebuild());
    }
}

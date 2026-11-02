//! Intake: incoming work proposals that are not tasks yet.
//!
//! An agent, a human or an integration proposes an [`IntakeItem`]. A member
//! triages it once: accepting links it to a thread (a new one, or one that
//! already exists), and rejecting records why. A triaged item stays put.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::ids::{MemberId, ThreadId, WorkspaceId};

/// Who or what proposed an intake item.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum IntakeSource {
    Agent,
    Human,
    Integration,
}

impl IntakeSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Agent => "agent",
            Self::Human => "human",
            Self::Integration => "integration",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "agent" => Some(Self::Agent),
            "human" => Some(Self::Human),
            "integration" => Some(Self::Integration),
            _ => None,
        }
    }
}

/// Where an intake item is in triage. `proposed` is the only state that
/// can change; `accepted` and `rejected` are final.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum IntakeState {
    Proposed,
    Accepted,
    Rejected,
}

impl IntakeState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Proposed => "proposed",
            Self::Accepted => "accepted",
            Self::Rejected => "rejected",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "proposed" => Some(Self::Proposed),
            "accepted" => Some(Self::Accepted),
            "rejected" => Some(Self::Rejected),
            _ => None,
        }
    }
}

/// An incoming work proposal in a workspace.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct IntakeItem {
    pub id: Uuid,
    pub workspace_id: WorkspaceId,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    pub source: IntakeSource,
    pub proposed_by: MemberId,
    pub state: IntakeState,
    /// The thread an accepted item became or was linked to.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<ThreadId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub triaged_by: Option<MemberId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub triaged_at: Option<DateTime<Utc>>,
    /// Why a rejected item was turned down.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reject_reason: Option<String>,
    pub created_at: DateTime<Utc>,
}

/// A new intake item.
#[derive(Debug, Clone)]
pub struct NewIntakeItem {
    pub workspace_id: WorkspaceId,
    pub title: String,
    pub body: Option<String>,
    pub source: IntakeSource,
    pub proposed_by: MemberId,
}

/// A triage decision on a proposed item.
#[derive(Debug, Clone, PartialEq)]
pub enum IntakeDecision {
    /// Accept, linking the item to this thread.
    Accept { thread_id: ThreadId },
    /// Reject, with a non-empty reason.
    Reject { reason: String },
}

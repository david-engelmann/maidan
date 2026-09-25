//! Required reviewers.
//!
//! A thread declares a **review requirement** — `required_count` (`k`) distinct
//! approvals — optionally from a **named reviewer set** (`n`). A reviewer
//! submits an [`ReviewDecision`] (approve / request-changes). The FSM
//! close-gate then refuses `closed` until `k` distinct **qualifying** approvals
//! exist — an approval qualifies when the reviewer is neither the thread's
//! `owner` nor its `assignee` (separation of duties) and, when a named set
//! exists, is in it — **and** no unresolved `refutes` edge blocks the thread.
//! This is a **gate**, not a poll/closer.
//!
//! A delivered `example.review.result/1` with any `critical` finding is fed in
//! as [`ReviewDecision::RequestChanges`] from a member who has
//! declared [`REVIEW_SKILL`]. That is a producer→reviewer adapter, not a new
//! gate: the close-gate still reads this table.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::{MemberId, ThreadId};

/// The member-skill tag a review agent declares. the adapter only writes
/// [`ReviewDecision::RequestChanges`] when the reviewer has this skill — so a
/// result from an implementer who is not review-skilled never arms the
/// close-gate.
pub const REVIEW_SKILL: &str = "review";

/// A reviewer's decision on a thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum ReviewDecision {
    Approve,
    RequestChanges,
}

impl ReviewDecision {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Approve => "approve",
            Self::RequestChanges => "request_changes",
        }
    }
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "approve" => Some(Self::Approve),
            "request_changes" => Some(Self::RequestChanges),
            _ => None,
        }
    }
}

/// A thread's review requirement: `required_count` distinct qualifying approvals.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ThreadReviewRequirement {
    pub thread_id: ThreadId,
    pub required_count: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A reviewer's decision record (one per `(thread, reviewer)` — re-submitting
/// changes it).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ThreadReview {
    pub thread_id: ThreadId,
    pub reviewer_id: MemberId,
    pub decision: ReviewDecision,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// The delegate that actually submitted this review for `reviewer_id`, when
    /// one did. `None`: the reviewer submitted it itself. A delegate that owns
    /// or worked the thread is not counted, whoever it reviews as.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_id: Option<MemberId>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Set on an approval when a change request sent the thread back for
    /// rework: it approved a version that no longer stands, so it no longer
    /// counts. Re-submitting the review clears it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dismissed_at: Option<DateTime<Utc>>,
}

/// The computed review standing of a thread — what the close-gate reads for the
/// **approval** side (the `refutes`-edge block is checked separately at the gate).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ReviewStatus {
    /// Approvals required (0 when no requirement is set).
    pub required_count: i64,
    /// Distinct **qualifying** approvals: decision = approve, reviewer is neither
    /// owner nor assignee, and (when a named reviewer set exists) is in it.
    pub approvals: i64,
    /// Whether the approval requirement is met (`required_count == 0` or
    /// `approvals >= required_count`).
    pub approvals_met: bool,
}

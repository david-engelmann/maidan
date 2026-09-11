//! Member freeze — the kill-switch record (Cluster 372, Wave 2 #20, G17/B25).
//!
//! The presence of a [`MemberFreeze`] row freezes a member: `claim_next` refuses
//! them, and freezing drops their active leases (releases their claimed threads).
//! A frozen member stays frozen until an operator explicitly unfreezes — the
//! freeze *is* the gate. This is **not** G4 PAUSE (which pauses a thread or
//! workspace); it stops one member's participation.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::MemberId;

/// A frozen member. `frozen_by` is the operator/member who applied the freeze;
/// `reason` is a free-text note for the audit trail.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MemberFreeze {
    pub member_id: MemberId,
    pub frozen_at: DateTime<Utc>,
    pub frozen_by: MemberId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

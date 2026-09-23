//! Durable authority for one workspace member to act for another.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{DelegationGrantId, MemberId, WorkspaceId};

pub const DELEGATED_TOKEN_DEFAULT_TTL_SECS: i64 = 15 * 60;
pub const DELEGATED_TOKEN_MAX_TTL_SECS: i64 = 60 * 60;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DelegationGrant {
    pub id: DelegationGrantId,
    pub workspace_id: WorkspaceId,
    pub subject_id: MemberId,
    pub delegate_id: MemberId,
    pub capabilities: Vec<String>,
    pub purpose: String,
    pub authorized_by: MemberId,
    pub expires_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewDelegationGrant {
    pub workspace_id: WorkspaceId,
    pub subject_id: MemberId,
    pub delegate_id: MemberId,
    pub capabilities: Vec<String>,
    pub purpose: String,
    pub authorized_by: MemberId,
    pub expires_at: DateTime<Utc>,
}

/// Who performed an action, and on whose behalf.
///
/// For a member acting for itself, `actor_id == subject_id` and there is no
/// grant. For a delegated action, `actor_id` is the delegate that really acted,
/// `subject_id` is the member it acted for, and `grant_id` is the authority it
/// used. Work done outside any request — scheduled sweeps, background workers —
/// has no attribution at all, which is how "the system did this" is recorded.
///
/// Stored inside an event's payload, so the event hash covers it: changing who
/// did something breaks the chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attribution {
    pub actor_id: MemberId,
    pub subject_id: MemberId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grant_id: Option<DelegationGrantId>,
}

impl Attribution {
    /// Whether the actor was acting for someone other than itself.
    pub fn is_delegated(&self) -> bool {
        self.grant_id.is_some()
    }
}

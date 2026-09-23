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

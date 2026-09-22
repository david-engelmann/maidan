//! Time-boxed, read-only capability tickets for sharing one channel and an
//! explicit set of workspace-linked artifacts outside the workspace identity
//! boundary.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{ChannelId, MemberId, ShareTicketId, WorkspaceId};

/// Hard ceiling for a cross-organization share ticket: 48 hours.
pub const SHARE_TICKET_MAX_TTL_SECS: i64 = 48 * 60 * 60;
/// A ticket stays a small, reviewable grant rather than a bulk export.
pub const SHARE_TICKET_MAX_ARTIFACTS: usize = 100;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ShareTicket {
    pub id: ShareTicketId,
    pub workspace_id: WorkspaceId,
    pub channel_id: ChannelId,
    /// Internal member accountable for the external share.
    pub owner_id: MemberId,
    pub created_by: MemberId,
    #[serde(skip)]
    #[cfg_attr(feature = "openapi", schema(ignore))]
    pub token_hash: String,
    pub expires_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewShareTicket {
    pub workspace_id: WorkspaceId,
    pub channel_id: ChannelId,
    pub owner_id: MemberId,
    pub created_by: MemberId,
    pub token_hash: String,
    pub expires_at: DateTime<Utc>,
    pub artifact_shas: Vec<String>,
}

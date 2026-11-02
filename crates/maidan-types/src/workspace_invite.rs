//! Invites that bring a person into an existing workspace as a human member.
//!
//! An invite is minted by a workspace admin, bound to one email and one role,
//! kept only as a hash of its secret, and consumed by the one OIDC sign-in
//! whose verified email matches it.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{MaidanSession, Member, MemberId, OidcIdentity, WorkspaceId, WorkspaceInviteId};

/// How long an invite lives when the request names no expiry: seven days.
pub const WORKSPACE_INVITE_DEFAULT_TTL_SECS: i64 = 7 * 24 * 60 * 60;
/// The longest an invite may live: thirty days.
pub const WORKSPACE_INVITE_MAX_TTL_SECS: i64 = 30 * 24 * 60 * 60;
/// Every invite secret starts with this, so the sign-up route can tell it
/// from a waitlist invite before looking either up.
pub const WORKSPACE_INVITE_SECRET_PREFIX: &str = "maid_inv_";

/// What an accepted invite lets the person do. Ordered: `Viewer < Member`,
/// so "no higher than the inviter" is a plain comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub enum WorkspaceInviteRole {
    // Reads the workspace in the console; posts nothing.
    Viewer,
    // Reads and writes in the console like any signed-in human.
    Member,
}

impl WorkspaceInviteRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Viewer => "viewer",
            Self::Member => "member",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "viewer" => Some(Self::Viewer),
            "member" => Some(Self::Member),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WorkspaceInvite {
    pub id: WorkspaceInviteId,
    pub workspace_id: WorkspaceId,
    /// The email the invite is bound to, lowercased.
    pub email: String,
    pub role: WorkspaceInviteRole,
    pub created_by: MemberId,
    #[serde(skip)]
    #[cfg_attr(feature = "openapi", schema(ignore))]
    pub token_hash: String,
    pub expires_at: DateTime<Utc>,
    pub accepted_at: Option<DateTime<Utc>>,
    /// The member the accepting sign-in created.
    pub accepted_member_id: Option<MemberId>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewWorkspaceInvite {
    pub workspace_id: WorkspaceId,
    pub email: String,
    pub role: WorkspaceInviteRole,
    pub created_by: MemberId,
    pub token_hash: String,
    pub expires_at: DateTime<Utc>,
}

/// One accepting sign-in: the identity the provider vouched for and the
/// handle the new member gets.
#[derive(Debug, Clone)]
pub struct AcceptWorkspaceInvite {
    pub invite_id: WorkspaceInviteId,
    pub workspace_id: WorkspaceId,
    /// The provider's verified email; must equal the invite's, ignoring case.
    pub email: String,
    pub issuer: String,
    pub subject: String,
    pub handle: String,
    pub session_expires_at: DateTime<Utc>,
}

/// What one accepted invite produced, in one commit.
#[derive(Debug, Clone)]
pub struct WorkspaceInviteAcceptance {
    pub invite: WorkspaceInvite,
    pub member: Member,
    pub identity: OidcIdentity,
    pub session: MaidanSession,
}

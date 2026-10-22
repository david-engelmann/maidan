//! Workspace suspension, the hosted service's abuse control: an operator
//! suspends a workspace with a reason, and while it stands every credential
//! of the workspace is refused and its outbound and background work stops.
//! Its data is kept, and lifting the suspension restores access.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{MemberId, WorkspaceId};

/// The refusal every surface gives a suspended workspace's credential.
pub const WORKSPACE_SUSPENDED: &str = "workspace suspended";

/// A suspension in force.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WorkspaceSuspension {
    pub workspace_id: WorkspaceId,
    pub reason: String,
    /// The operator who suspended it.
    pub suspended_by: Option<MemberId>,
    pub suspended_at: DateTime<Utc>,
}

/// The body of `PUT /operator/workspaces/{id}/suspension`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SuspendWorkspace {
    /// Why, for the audit log and for the workspace's own operators.
    pub reason: String,
}

/// The answer to `DELETE /operator/workspaces/{id}/suspension`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct LiftedSuspension {
    /// `false` when the workspace was not suspended.
    pub lifted: bool,
}

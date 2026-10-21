//! Holding back a suspended workspace's outbound and background work. The
//! change sweep, Linear notices, the scheduler and the egress, mail and web
//! push workers ask here before acting for a workspace. A queued delivery is
//! deferred, not dropped, so lifting the suspension lets it go out.

use maidan_types::{MemberId, WorkspaceId};

use crate::state::AppState;

/// How long a suspended workspace's queued delivery waits before it is
/// looked at again.
pub const SUSPENDED_DEFER: chrono::Duration = chrono::Duration::minutes(5);

/// Whether work for `workspace_id` is held back. A failed lookup holds it back
/// too: work for a workspace that may be suspended waits for the next pass.
pub async fn held_back(state: &AppState, workspace_id: WorkspaceId) -> bool {
    match state.store.get_workspace_suspension(workspace_id).await {
        Ok(suspension) => suspension.is_some(),
        Err(err) => {
            tracing::warn!(error = %err, workspace_id = %workspace_id.0, "suspension check failed; holding work back");
            true
        }
    }
}

/// [`held_back`] for the workspace of `member_id`.
pub async fn member_held_back(state: &AppState, member_id: MemberId) -> bool {
    match state.store.get_member(member_id).await {
        Ok(member) => held_back(state, member.workspace_id).await,
        Err(err) => {
            tracing::warn!(error = %err, member_id = %member_id.0, "member lookup failed; holding work back");
            true
        }
    }
}

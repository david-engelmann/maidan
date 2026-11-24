//! What a workspace's own inbound URLs share: the secret a delivery is
//! verified with.
//!
//! A route on `/workspaces/{wid}/connections/{surface}/...` carries no bearer.
//! The only thing that lets a delivery in is the signature under the secret
//! the workspace stored when it connected the surface, so a workspace that
//! does not exist, one with no connection and one with no secret all answer
//! the same `404`.

use maidan_types::{ConnectionSurface, WorkspaceId};

use crate::state::AppState;

/// The inbound secret `workspace_id` stored for `surface`, or `None` when it
/// stored none, has no such connection, does not exist, or the connection
/// cannot be opened. A caller must answer all of those alike.
pub(crate) async fn stored_inbound_secret(
    state: &AppState,
    workspace_id: WorkspaceId,
    surface: ConnectionSurface,
) -> Option<String> {
    match state.stored_connection(workspace_id, surface).await {
        Ok(Some((_, connection))) => connection.inbound_secret(surface).map(str::to_string),
        Ok(None) => None,
        Err(err) => {
            tracing::warn!(error = %err, %surface, "inbound delivery: the connection could not be read");
            None
        }
    }
}

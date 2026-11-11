//! The authority a registration acts with: the token that made it.
//!
//! FSM hooks, outbound webhooks (the mention webhook included), slash
//! commands and A2A push configs each act for whoever registered them, so
//! each may do and see no more than its creator, and each stops when its
//! creator does (SEC-013, SEC-017).

use maidan_auth::AuthContext;
use maidan_types::{ApiTokenId, Event, WorkspaceId};

use crate::state::AppState;

/// The creator's authority, resolved now as a bearer of `token` would get
/// it. `Err` is why the registration does not fire: its token was revoked,
/// expired or rotated away, its member was deactivated, the token no longer
/// holds `required` or belongs to another workspace, or none was recorded.
/// Only a server with auth disabled registers without a token, and only
/// there does such a registration fire, with that server's authority.
pub async fn creator_authority(
    state: &AppState,
    workspace_id: WorkspaceId,
    token: Option<ApiTokenId>,
    required: &str,
) -> Result<AuthContext, &'static str> {
    let Some(token_id) = token else {
        return if state.auth_disabled {
            Ok(AuthContext::bypass())
        } else {
            Err("registered without a token")
        };
    };
    let auth = match maidan_auth::resolve_token_id(state.store.as_ref(), token_id).await {
        Ok(auth) => auth,
        Err(maidan_auth::AuthError::Forbidden(_)) => return Err("workspace suspended"),
        // A revoked, expired or rotated token, or a deactivated member, is no
        // longer found among the live ones.
        Err(
            maidan_auth::AuthError::Unauthorized
            | maidan_auth::AuthError::Store(maidan_store::StoreError::NotFound),
        ) => return Err("creator token no longer valid"),
        Err(maidan_auth::AuthError::Store(_)) => return Err("creator check failed"),
    };
    if auth.workspace_id != workspace_id {
        return Err("creator token is for another workspace");
    }
    if auth.require_capability(required).is_err() {
        return Err(match required {
            maidan_auth::capability::WORKSPACE_ADMIN => "creator no longer holds workspace:admin",
            _ => "creator no longer holds workspace:write",
        });
    }
    Ok(auth)
}

/// Whether the creator may read where `event` happened: its thread (a DM
/// only for a participant), else its channel. An event with neither is
/// workspace-wide. A lookup that fails hides the event.
pub async fn creator_can_see(state: &AppState, auth: &AuthContext, event: &Event) -> bool {
    if auth.bypass {
        return true;
    }
    let store = state.store.as_ref();
    if let Some(thread_id) = event.thread_id() {
        match maidan_auth::can_access_thread(store, auth, thread_id).await {
            Ok(seen) => return seen,
            // A thread gone since the event falls back to its channel.
            Err(maidan_auth::AuthError::Store(maidan_store::StoreError::NotFound)) => {}
            Err(_) => return false,
        }
    }
    if let Some(channel_id) = event.channel_id() {
        return maidan_auth::can_access_channel(store, auth, channel_id)
            .await
            .unwrap_or(false);
    }
    // A DM event names its thread; one that names only its conversation
    // cannot be checked, so it is not sent.
    event.dm_conversation_id().is_none()
}

//! The ways a caller that presented no bearer becomes an [`AuthContext`].
//!
//! [`AuthContext`] can be built only in this crate, so each path that is not a
//! bearer gets its own resolver here, and each reads the store record that
//! proves the caller before it hands one out.

use chrono::Utc;
use maidan_store::Store;
use maidan_types::{ChannelId, MaidanSession, MemberId, SessionId, ThreadId, WorkspaceId};

use crate::context::AuthContext;
use crate::error::AuthError;

/// A browser session found in the store and not yet expired. Only
/// [`resolve_session`] makes one, so holding one shows the row exists.
#[derive(Debug, Clone)]
pub struct SessionPrincipal {
    session: MaidanSession,
}

impl SessionPrincipal {
    /// The stored session this principal was read from.
    pub fn session(&self) -> &MaidanSession {
        &self.session
    }

    /// The caller this session acts as on a route that grants `capabilities`.
    /// A session made from a token carries that token's authority instead,
    /// resolved through [`crate::resolve_token_id`].
    pub fn context(&self, capabilities: Vec<String>) -> AuthContext {
        AuthContext::from_session(
            self.session.member_id,
            self.session.workspace_id,
            capabilities,
        )
    }
}

/// The session `session_id` names, if it is stored and unexpired. An expired
/// row is deleted on the way out, so the next request does not read it again.
pub async fn resolve_session(
    store: &dyn Store,
    session_id: SessionId,
) -> Result<SessionPrincipal, AuthError> {
    let session = store
        .get_session(session_id)
        .await
        .map_err(|_| AuthError::Unauthorized)?;
    if session.expires_at < Utc::now() {
        let _ = store.delete_expired_session(session.id).await;
        return Err(AuthError::Unauthorized);
    }
    Ok(SessionPrincipal { session })
}

/// The name prefix of the only workspaces a caller with no credential may read.
pub const SYNTHETIC_WORKSPACE_PREFIX: &str = "synthetic-";

/// The anonymous reader of `workspace_id`: `workspace:read` and nothing else,
/// as a member id that belongs to nobody, fresh each call. Refused unless the
/// workspace exists and is named [`SYNTHETIC_WORKSPACE_PREFIX`]…, so a
/// misconfigured dev flag can never open a real workspace.
pub async fn resolve_anonymous_reader(
    store: &dyn Store,
    workspace_id: WorkspaceId,
) -> Result<AuthContext, AuthError> {
    let workspace = store.get_workspace(workspace_id).await?;
    if !workspace.name.starts_with(SYNTHETIC_WORKSPACE_PREFIX) {
        return Err(AuthError::Forbidden(format!(
            "workspace `{}` is not named {SYNTHETIC_WORKSPACE_PREFIX}…, and only a synthetic \
             workspace is read without a credential",
            workspace.name
        )));
    }
    // An id that belongs to nobody: the foreign keys refuse to attach a
    // channel membership, DM, grant or role to a member that does not exist.
    Ok(AuthContext::anonymous_reader(MemberId::new(), workspace_id))
}

/// Whether `member_id`, a member of `workspace_id`, can read `thread_id` now.
/// For a recipient chosen by the server rather than a caller (a follower, a
/// mention): it reads with no capability, so only the thread and channel
/// rules decide. The caller checks the member belongs to the workspace.
pub async fn member_can_access_thread(
    store: &dyn Store,
    workspace_id: WorkspaceId,
    member_id: MemberId,
    thread_id: ThreadId,
) -> Result<bool, AuthError> {
    let auth = AuthContext::from_session(member_id, workspace_id, Vec::new());
    crate::can_access_thread(store, &auth, thread_id).await
}

/// Whether `member_id`, a member of `workspace_id`, can read `channel_id` now.
/// For a member the server acts for (a schedule's creator) rather than a
/// caller: with no capability, so only the channel's membership rule decides.
pub async fn member_can_access_channel(
    store: &dyn Store,
    workspace_id: WorkspaceId,
    member_id: MemberId,
    channel_id: ChannelId,
) -> Result<bool, AuthError> {
    let auth = AuthContext::from_session(member_id, workspace_id, Vec::new());
    crate::can_access_channel(store, &auth, channel_id).await
}

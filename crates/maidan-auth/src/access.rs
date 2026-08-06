//! Per-channel authorization (Cluster 160).
//!
//! Public channels are open to the whole workspace (no `channel_members` rows
//! required); private channels are gated to explicit members. The DM system
//! channel (`__dm__`) is exempt — DM and group-DM membership is enforced
//! per-conversation by the `dm` / `group_dm` participant checks, which remain
//! authoritative. `bypass` callers (auth disabled / tests) always pass, matching
//! `has_capability` / `ensure_workspace`.

use maidan_store::Store;
use maidan_types::{ChannelId, MessageId, ThreadId, DM_CHANNEL_NAME};

use crate::{AuthContext, AuthError};

/// Ensure the caller may access `channel_id`. Verifies the workspace first
/// (tenant isolation), then — for a private, non-DM channel — that the caller
/// has a `channel_members` row.
pub async fn ensure_channel_access(
    store: &dyn Store,
    auth: &AuthContext,
    channel_id: ChannelId,
) -> Result<(), AuthError> {
    if auth.bypass {
        return Ok(());
    }
    let channel = store.get_channel(channel_id).await?;
    auth.ensure_workspace(channel.workspace_id)?;
    if !channel.private || channel.name == DM_CHANNEL_NAME {
        return Ok(());
    }
    if store.channel_is_member(channel_id, auth.member_id).await? {
        Ok(())
    } else {
        Err(AuthError::Forbidden(
            "caller is not a member of this private channel".into(),
        ))
    }
}

/// The bool form of [`ensure_channel_access`], for filtering result sets
/// (e.g. search hits) rather than short-circuiting a request. `Forbidden`
/// becomes `false`; store errors still propagate.
pub async fn can_access_channel(
    store: &dyn Store,
    auth: &AuthContext,
    channel_id: ChannelId,
) -> Result<bool, AuthError> {
    match ensure_channel_access(store, auth, channel_id).await {
        Ok(()) => Ok(true),
        Err(AuthError::Forbidden(_)) => Ok(false),
        Err(e) => Err(e),
    }
}

/// Ensure the caller may access the channel that owns `thread_id`.
pub async fn ensure_thread_access(
    store: &dyn Store,
    auth: &AuthContext,
    thread_id: ThreadId,
) -> Result<(), AuthError> {
    if auth.bypass {
        return Ok(());
    }
    let thread = store.get_thread(thread_id).await?;
    ensure_channel_access(store, auth, thread.channel_id).await
}

/// Ensure the caller may access the channel that owns `message_id`.
pub async fn ensure_message_access(
    store: &dyn Store,
    auth: &AuthContext,
    message_id: MessageId,
) -> Result<(), AuthError> {
    if auth.bypass {
        return Ok(());
    }
    let message = store.get_message(message_id).await?;
    ensure_thread_access(store, auth, message.thread_id).await
}

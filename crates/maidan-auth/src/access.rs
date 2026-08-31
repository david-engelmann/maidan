//! Per-channel authorization (Cluster 160).
//!
//! Public channels are open to the whole workspace (no `channel_members` rows
//! required); private channels are gated to explicit members. The DM system
//! channel (`__dm__`) is exempt — DM and group-DM membership is enforced
//! per-conversation by the `dm` / `group_dm` participant checks, which remain
//! authoritative. `bypass` callers (auth disabled / tests) always pass, matching
//! `has_capability` / `ensure_workspace`.

use maidan_store::Store;
use maidan_types::{ChannelId, MessageId, ThreadId, WorkspaceId, DM_CHANNEL_NAME};

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

/// The private (non-DM) channels in `workspace_id` the caller is **not** a member
/// of (Cluster 200) — the set to exclude at the query level so inaccessible hits
/// don't crowd out a search's requested `limit`. `bypass` callers get an empty
/// set (they see everything). DM threads are intentionally excluded: DM access is
/// participant-based, not channel-membership, so it stays with the authoritative
/// thread-level post-filter. This is a *pre-filter*, never the sole check.
pub async fn private_channel_deny_set(
    store: &dyn Store,
    auth: &AuthContext,
    workspace_id: WorkspaceId,
) -> Result<Vec<ChannelId>, AuthError> {
    if auth.bypass {
        return Ok(Vec::new());
    }
    let channels = store.list_channels(workspace_id).await?;
    let mut deny = Vec::new();
    for channel in channels {
        if channel.private
            && channel.name != DM_CHANNEL_NAME
            && !store.channel_is_member(channel.id, auth.member_id).await?
        {
            deny.push(channel.id);
        }
    }
    Ok(deny)
}

/// Ensure the caller participates in the DM / group-DM conversation backing
/// `thread_id` (Cluster 180). DM threads all live in the one `__dm__` system
/// channel, so `ensure_channel_access` can't gate them per-conversation — this
/// resolves the specific conversation and checks membership. A `__dm__` thread
/// with no resolvable conversation is treated as inaccessible (defensive).
pub async fn ensure_dm_participant(
    store: &dyn Store,
    auth: &AuthContext,
    thread_id: ThreadId,
) -> Result<(), AuthError> {
    let me = auth.member_id;
    if let Some(dm) = store.dm_conversation_for_thread(thread_id).await? {
        if dm.member_low_id == me || dm.member_high_id == me {
            return Ok(());
        }
    } else if let Some(gdm) = store.group_dm_conversation_for_thread(thread_id).await? {
        if gdm.member_ids.contains(&me) {
            return Ok(());
        }
    }
    Err(AuthError::Forbidden(
        "caller is not a participant of this DM conversation".into(),
    ))
}

/// A thread's resolved location, returned by [`authorize_thread`]. Mirrors the
/// router's `ThreadContext` fields so a handler that previously kept a `ctx` from
/// `resolve_thread_context` reads it unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ThreadScope {
    pub workspace_id: WorkspaceId,
    pub channel_id: ChannelId,
    pub thread_id: ThreadId,
}

/// Resolve a thread's workspace + channel **and** authorize the caller in a
/// single pass (Cluster 339). Most write handlers used to call
/// `resolve_thread_context` (get_thread + get_channel) and then
/// [`ensure_thread_access`] (the same two fetches again); this does the fetch
/// once and returns the [`ThreadScope`] the handler needs. The access rule is
/// identical to [`ensure_thread_access`] — workspace isolation, then
/// DM-conversation participation for a `__dm__` thread or `channel_members` for a
/// private channel. `bypass` skips the checks but still returns the scope (the
/// handler needs the location regardless).
pub async fn authorize_thread(
    store: &dyn Store,
    auth: &AuthContext,
    thread_id: ThreadId,
) -> Result<ThreadScope, AuthError> {
    let thread = store.get_thread(thread_id).await?;
    let channel = store.get_channel(thread.channel_id).await?;
    let scope = ThreadScope {
        workspace_id: channel.workspace_id,
        channel_id: channel.id,
        thread_id,
    };
    if auth.bypass {
        return Ok(scope);
    }
    auth.ensure_workspace(channel.workspace_id)?;
    if channel.name == DM_CHANNEL_NAME {
        ensure_dm_participant(store, auth, thread_id).await?;
    } else if channel.private && !store.channel_is_member(channel.id, auth.member_id).await? {
        return Err(AuthError::Forbidden(
            "caller is not a member of this private channel".into(),
        ));
    }
    Ok(scope)
}

/// Ensure the caller may access `thread_id`. For a normal channel this is
/// channel access; for a `__dm__` thread it is DM-conversation participation
/// (Cluster 180) — closing the gap where a DM thread was readable via the
/// generic thread route by any workspace member. Delegates to
/// [`authorize_thread`] so the rule stays single-sourced (Cluster 339).
pub async fn ensure_thread_access(
    store: &dyn Store,
    auth: &AuthContext,
    thread_id: ThreadId,
) -> Result<(), AuthError> {
    if auth.bypass {
        return Ok(());
    }
    authorize_thread(store, auth, thread_id).await.map(|_| ())
}

/// The bool form of [`ensure_thread_access`], for filtering aggregate result
/// sets (search hits, workspace-context threads) per-thread (Cluster 180) — this
/// is DM-participant-aware, unlike the channel-keyed [`can_access_channel`],
/// which exempts the shared `__dm__` channel and so leaked DM content into
/// aggregate reads. `Forbidden` becomes `false`; store errors propagate.
pub async fn can_access_thread(
    store: &dyn Store,
    auth: &AuthContext,
    thread_id: ThreadId,
) -> Result<bool, AuthError> {
    match ensure_thread_access(store, auth, thread_id).await {
        Ok(()) => Ok(true),
        Err(AuthError::Forbidden(_)) => Ok(false),
        Err(e) => Err(e),
    }
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

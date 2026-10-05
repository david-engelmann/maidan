//! Member handlers: list/get members, mentions-for-member, and inbox.

use axum::http::StatusCode;
use axum::{extract::State, Extension, Json};
use maidan_auth::{capability::WORKSPACE_READ, AuthContext};
use maidan_types::*;

use super::{cap, ensure_own_personal_state, ensure_workspace, requested_member, ApiResult};
use crate::dto::*;
use crate::error::ApiError;
use crate::extract::{ApiJson, ApiPath, ApiQuery};
use crate::state::AppState;

/// `GET /me` — the caller's own identity. Reflects the request's auth
/// (member/workspace/capabilities), so an agent handed only a base URL + token
/// can discover which member the server will attribute writes to.
/// `workspace:read`.
pub async fn get_me(Extension(auth): Extension<AuthContext>) -> ApiResult<Json<WhoAmI>> {
    cap(&auth, WORKSPACE_READ)?;
    Ok(Json(WhoAmI {
        actor_id: auth.actor_id.0,
        member_id: auth.member_id.0,
        delegation_grant_id: auth.delegation_grant_id.map(|id| id.0),
        workspace_id: auth.workspace_id.0,
        capabilities: auth.capabilities().to_vec(),
        is_bearer: auth.token_id.is_some(),
        token_id: auth.token_id.map(|id| id.0),
        known_capabilities: maidan_auth::capability::all(),
        capability_sets: maidan_auth::held_sets(auth.capabilities()),
    }))
}

#[cfg(feature = "bootstrap")]
pub async fn create_member(
    State(state): State<AppState>,
    ApiPath(workspace_id): ApiPath<uuid::Uuid>,
    ApiJson(body): ApiJson<CreateMember>,
) -> ApiResult<(StatusCode, Json<Member>)> {
    let (m, stored) = state
        .store
        .create_member_with_event(NewMember {
            workspace_id: WorkspaceId(workspace_id),
            handle: body.handle,
            display_name: body.display_name,
            kind: body.kind,
        })
        .await?;
    super::publish_stored(&state, stored).await;
    Ok((StatusCode::CREATED, Json(m)))
}

pub async fn list_members(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(workspace_id): ApiPath<uuid::Uuid>,
) -> ApiResult<Json<Vec<Member>>> {
    let workspace_id = WorkspaceId(workspace_id);
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, workspace_id)?;
    Ok(Json(state.store.list_members(workspace_id).await?))
}

pub async fn get_member(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
) -> ApiResult<Json<Member>> {
    let member = requested_member(&state, &auth, MemberId(id)).await?;
    cap(&auth, WORKSPACE_READ)?;
    Ok(Json(member))
}

pub async fn list_mentions_for_member(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
    ApiQuery(q): ApiQuery<ListMentionsQuery>,
) -> ApiResult<Json<Vec<Mention>>> {
    requested_member(&state, &auth, MemberId(id)).await?;
    cap(&auth, WORKSPACE_READ)?;
    // A member's mentions are their own. No caller — session or bearer — reads
    // another's; an orchestrator acting for an agent holds a delegated token
    // that *is* that agent.
    ensure_own_personal_state(&auth, MemberId(id))?;
    Ok(Json(
        state
            .store
            .list_mentions_for_member(MemberId(id), q.limit.clamp(1, 500))
            .await?,
    ))
}

pub async fn get_member_inbox(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
    ApiQuery(q): ApiQuery<ListInboxQuery>,
) -> ApiResult<Json<MemberInbox>> {
    requested_member(&state, &auth, MemberId(id)).await?;
    cap(&auth, WORKSPACE_READ)?;
    // Defensive self-only. See list_mentions_for_member.
    ensure_own_personal_state(&auth, MemberId(id))?;
    Ok(Json(
        state
            .store
            .list_member_inbox(MemberId(id), q.limit.clamp(1, 500))
            .await?,
    ))
}

pub async fn mark_member_inbox_read(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
    ApiJson(body): ApiJson<MarkInboxRead>,
) -> ApiResult<Json<MemberInbox>> {
    requested_member(&state, &auth, MemberId(id)).await?;
    cap(&auth, WORKSPACE_READ)?;
    // `advance_inbox_last_read_at` writes one member's read marker, so only
    // that member may move it.
    ensure_own_personal_state(&auth, MemberId(id))?;
    state
        .store
        .advance_inbox_last_read_at(MemberId(id), body.read_through)
        .await?;
    Ok(Json(state.store.list_member_inbox(MemberId(id), 50).await?))
}

/// A member's per-recipient notifications, newest first. Self-only for every
/// caller.
pub async fn list_member_notifications(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
    ApiQuery(q): ApiQuery<ListNotificationsQuery>,
) -> ApiResult<Json<Vec<Notification>>> {
    cap(&auth, WORKSPACE_READ)?;
    requested_member(&state, &auth, MemberId(id)).await?;
    ensure_own_personal_state(&auth, MemberId(id))?;
    let limit = q.limit.clamp(1, 500);
    Ok(Json(
        state
            .store
            .list_notifications(MemberId(id), q.unread_only, limit)
            .await?,
    ))
}

/// A member's notifications collapsed into per-thread groups, newest-activity
/// first — so a busy thread shows as one row instead of flooding the flat
/// inbox. `limit` bounds how many notifications are scanned (default 200, clamp
/// 1..=500); `unread_only` groups only unread ones. Self-only for a session
/// caller.
pub async fn list_member_notifications_grouped(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
    ApiQuery(q): ApiQuery<ListNotificationsQuery>,
) -> ApiResult<Json<Vec<NotificationThreadGroup>>> {
    cap(&auth, WORKSPACE_READ)?;
    requested_member(&state, &auth, MemberId(id)).await?;
    ensure_own_personal_state(&auth, MemberId(id))?;
    let limit = q.limit.clamp(1, 500);
    let notes = state
        .store
        .list_notifications(MemberId(id), q.unread_only, limit)
        .await?;
    Ok(Json(maidan_types::group_notifications_by_thread(&notes)))
}

/// A member's buried decisions — task results produced by someone else in a
/// channel/thread the member follows, since `since` (default 7 days ago),
/// newest first. The queryable form of the buried-decisions digest. Self-only
/// for a session caller; `workspace:read`.
pub async fn list_member_decisions(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
    ApiQuery(q): ApiQuery<DecisionsQuery>,
) -> ApiResult<Json<Vec<BuriedDecision>>> {
    cap(&auth, WORKSPACE_READ)?;
    requested_member(&state, &auth, MemberId(id)).await?;
    ensure_own_personal_state(&auth, MemberId(id))?;
    let since = q
        .since
        .unwrap_or_else(|| chrono::Utc::now() - chrono::Duration::days(7));
    let limit = q.limit.unwrap_or(50).clamp(1, 200);
    let decisions = state
        .store
        .buried_decisions_for_member(MemberId(id), since, limit)
        .await?;
    Ok(Json(decisions))
}

/// Per-channel result/gate/stuck counts composed from this member's unread
/// followed-member notifications. Self-only for every caller: the digest is a
/// projection of whom the member follows, which is their own business.
pub async fn get_member_manager_digest(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
    ApiQuery(q): ApiQuery<ManagerDigestQuery>,
) -> ApiResult<Json<ManagerDigest>> {
    cap(&auth, WORKSPACE_READ)?;
    let member_id = MemberId(id);
    requested_member(&state, &auth, member_id).await?;
    ensure_own_personal_state(&auth, member_id)?;
    let since = q
        .since
        .unwrap_or_else(|| chrono::Utc::now() - chrono::Duration::days(7));
    Ok(Json(
        state
            .store
            .manager_digest_for_member(member_id, since)
            .await?,
    ))
}

/// A member's unread-notification badge count. Self-only for sessions.
pub async fn member_unread_notification_count(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
) -> ApiResult<Json<UnreadCount>> {
    cap(&auth, WORKSPACE_READ)?;
    requested_member(&state, &auth, MemberId(id)).await?;
    ensure_own_personal_state(&auth, MemberId(id))?;
    let count = state.store.unread_notification_count(MemberId(id)).await?;
    Ok(Json(UnreadCount { count }))
}

/// Mark one of a member's notifications read. Self-only for sessions; the store
/// scopes the write to `(member_id, id)`, so `404` when the notification isn't
/// this member's. Returns the new unread count.
pub async fn mark_member_notification_read(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath((id, nid)): ApiPath<(uuid::Uuid, uuid::Uuid)>,
) -> ApiResult<Json<UnreadCount>> {
    cap(&auth, WORKSPACE_READ)?;
    requested_member(&state, &auth, MemberId(id)).await?;
    ensure_own_personal_state(&auth, MemberId(id))?;
    if !state
        .store
        .mark_notification_read(MemberId(id), NotificationId(nid))
        .await?
    {
        return Err(ApiError::NotFound);
    }
    let count = state.store.unread_notification_count(MemberId(id)).await?;
    Ok(Json(UnreadCount { count }))
}

/// Snooze one notification until `until` — it drops out of the inbox + badge
/// until then. Self-only for a session caller; `404` when the notification
/// isn't this member's. Returns the new unread count.
pub async fn snooze_member_notification(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath((id, nid)): ApiPath<(uuid::Uuid, uuid::Uuid)>,
    ApiJson(body): ApiJson<SnoozeNotification>,
) -> ApiResult<Json<UnreadCount>> {
    cap(&auth, WORKSPACE_READ)?;
    requested_member(&state, &auth, MemberId(id)).await?;
    ensure_own_personal_state(&auth, MemberId(id))?;
    if !state
        .store
        .snooze_notification(MemberId(id), NotificationId(nid), body.until)
        .await?
    {
        return Err(ApiError::NotFound);
    }
    let count = state.store.unread_notification_count(MemberId(id)).await?;
    Ok(Json(UnreadCount { count }))
}

/// Mark all of a member's notifications read. Self-only for sessions.
pub async fn mark_all_member_notifications_read(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
) -> ApiResult<Json<MarkAllRead>> {
    cap(&auth, WORKSPACE_READ)?;
    requested_member(&state, &auth, MemberId(id)).await?;
    ensure_own_personal_state(&auth, MemberId(id))?;
    let cleared = state
        .store
        .mark_all_notifications_read(MemberId(id))
        .await? as i64;
    Ok(Json(MarkAllRead { cleared }))
}

/// Set a member's mute preference for an event kind. Self-only for every
/// caller.
pub async fn set_member_notification_pref(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
    ApiJson(body): ApiJson<SetNotificationPref>,
) -> ApiResult<Json<NotificationPref>> {
    cap(&auth, WORKSPACE_READ)?;
    requested_member(&state, &auth, MemberId(id)).await?;
    ensure_own_personal_state(&auth, MemberId(id))?;
    Ok(Json(
        state
            .store
            .set_notification_pref(MemberId(id), body.kind, body.muted)
            .await?,
    ))
}

/// A member's notification preferences. Self-only for sessions.
pub async fn list_member_notification_prefs(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
) -> ApiResult<Json<Vec<NotificationPref>>> {
    cap(&auth, WORKSPACE_READ)?;
    requested_member(&state, &auth, MemberId(id)).await?;
    ensure_own_personal_state(&auth, MemberId(id))?;
    Ok(Json(
        state.store.list_notification_prefs(MemberId(id)).await?,
    ))
}

/// Follow a channel to be notified of activity there. Self-only for a session
/// caller; requires access to the channel being followed.
pub async fn follow_member_channel(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
    ApiJson(body): ApiJson<FollowChannel>,
) -> ApiResult<StatusCode> {
    cap(&auth, WORKSPACE_READ)?;
    requested_member(&state, &auth, MemberId(id)).await?;
    ensure_own_personal_state(&auth, MemberId(id))?;
    maidan_auth::ensure_channel_access(state.store.as_ref(), &auth, body.channel_id).await?;
    state
        .store
        .follow_channel(MemberId(id), body.channel_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Unfollow a channel. Self-only for sessions; `404` if not following.
pub async fn unfollow_member_channel(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath((id, cid)): ApiPath<(uuid::Uuid, uuid::Uuid)>,
) -> ApiResult<StatusCode> {
    cap(&auth, WORKSPACE_READ)?;
    requested_member(&state, &auth, MemberId(id)).await?;
    ensure_own_personal_state(&auth, MemberId(id))?;
    if state
        .store
        .unfollow_channel(MemberId(id), ChannelId(cid))
        .await?
    {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

/// The channels a member follows. Self-only for sessions.
pub async fn list_member_channel_follows(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
) -> ApiResult<Json<Vec<ChannelFollow>>> {
    cap(&auth, WORKSPACE_READ)?;
    requested_member(&state, &auth, MemberId(id)).await?;
    ensure_own_personal_state(&auth, MemberId(id))?;
    Ok(Json(state.store.list_channel_follows(MemberId(id)).await?))
}

/// Follow a thread to be notified of activity there. Self-only for a session
/// caller; requires access to the thread being followed.
pub async fn follow_member_thread(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
    ApiJson(body): ApiJson<FollowThread>,
) -> ApiResult<StatusCode> {
    cap(&auth, WORKSPACE_READ)?;
    requested_member(&state, &auth, MemberId(id)).await?;
    ensure_own_personal_state(&auth, MemberId(id))?;
    maidan_auth::ensure_thread_access(state.store.as_ref(), &auth, body.thread_id).await?;
    state
        .store
        .follow_thread(MemberId(id), body.thread_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Unfollow a thread. Self-only for sessions; `404` if not following.
pub async fn unfollow_member_thread(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath((id, tid)): ApiPath<(uuid::Uuid, uuid::Uuid)>,
) -> ApiResult<StatusCode> {
    cap(&auth, WORKSPACE_READ)?;
    requested_member(&state, &auth, MemberId(id)).await?;
    ensure_own_personal_state(&auth, MemberId(id))?;
    if state
        .store
        .unfollow_thread(MemberId(id), ThreadId(tid))
        .await?
    {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

/// The threads a member follows. Self-only for sessions.
pub async fn list_member_thread_follows(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
) -> ApiResult<Json<Vec<ThreadFollow>>> {
    cap(&auth, WORKSPACE_READ)?;
    requested_member(&state, &auth, MemberId(id)).await?;
    ensure_own_personal_state(&auth, MemberId(id))?;
    Ok(Json(state.store.list_thread_follows(MemberId(id)).await?))
}

/// Follow another member's work occupancy. Self-only for sessions; both ends
/// must belong to the caller's workspace and self-follow is meaningless.
pub async fn follow_member_occupancy(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
    ApiJson(body): ApiJson<FollowMember>,
) -> ApiResult<StatusCode> {
    cap(&auth, WORKSPACE_READ)?;
    let follower_id = MemberId(id);
    let follower = requested_member(&state, &auth, follower_id).await?;
    ensure_own_personal_state(&auth, follower_id)?;
    if follower_id == body.followed_member_id {
        return Err(ApiError::BadRequest("a member cannot follow itself".into()));
    }
    state
        .store
        .get_member_in(follower.workspace_id, body.followed_member_id)
        .await?;
    state
        .store
        .follow_member(follower_id, body.followed_member_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Stop following another member's occupancy. Self-only for sessions.
pub async fn unfollow_member_occupancy(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath((id, followed_id)): ApiPath<(uuid::Uuid, uuid::Uuid)>,
) -> ApiResult<StatusCode> {
    cap(&auth, WORKSPACE_READ)?;
    let follower_id = MemberId(id);
    requested_member(&state, &auth, follower_id).await?;
    ensure_own_personal_state(&auth, follower_id)?;
    if state
        .store
        .unfollow_member(follower_id, MemberId(followed_id))
        .await?
    {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

/// The members whose work occupancy this member follows. Self-only for
/// sessions; the returned rows are subscription metadata, not presence state.
pub async fn list_member_occupancy_follows(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
) -> ApiResult<Json<Vec<MemberFollow>>> {
    cap(&auth, WORKSPACE_READ)?;
    let follower_id = MemberId(id);
    requested_member(&state, &auth, follower_id).await?;
    ensure_own_personal_state(&auth, follower_id)?;
    Ok(Json(state.store.list_member_follows(follower_id).await?))
}

/// A member's live occupancy: ephemeral presence plus assigned non-terminal
/// threads the caller can access. Same-workspace, `workspace:read`.
pub async fn get_member_occupancy(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
) -> ApiResult<Json<MemberOccupancy>> {
    cap(&auth, WORKSPACE_READ)?;
    let member_id = MemberId(id);
    let member = requested_member(&state, &auth, member_id).await?;
    let assigned = state
        .store
        .list_assigned_threads(member.workspace_id, member_id)
        .await?;
    let mut visible = Vec::with_capacity(assigned.len());
    for thread in assigned {
        if auth.bypass
            || maidan_auth::can_access_thread(state.store.as_ref(), &auth, thread.id).await?
        {
            visible.push(thread);
        }
    }
    let presence = match state.presence.status(member.workspace_id, member_id) {
        Some(crate::presence::PresenceStatus::Online) => OccupancyPresence::Online,
        Some(crate::presence::PresenceStatus::Away) => OccupancyPresence::Away,
        None => OccupancyPresence::Offline,
    };
    Ok(Json(MemberOccupancy {
        member_id,
        presence,
        assigned_threads: visible,
    }))
}

/// Set a member's delivery email address — opting in to email notifications.
/// Self-only for a session caller. A light `@` sanity check rejects obvious
/// garbage; the transport validates fully on send.
pub async fn set_member_email(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
    ApiJson(body): ApiJson<SetEmail>,
) -> ApiResult<Json<MemberEmail>> {
    cap(&auth, WORKSPACE_READ)?;
    requested_member(&state, &auth, MemberId(id)).await?;
    ensure_own_personal_state(&auth, MemberId(id))?;
    let email = body.email.trim();
    if !email.contains('@') || email.len() < 3 {
        return Err(ApiError::BadRequest("email must be a valid address".into()));
    }
    Ok(Json(
        state.store.set_member_email(MemberId(id), email).await?,
    ))
}

/// A member's delivery email, or `404` if unset. Self-only for sessions.
pub async fn get_member_email(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
) -> ApiResult<Json<MemberEmail>> {
    cap(&auth, WORKSPACE_READ)?;
    requested_member(&state, &auth, MemberId(id)).await?;
    ensure_own_personal_state(&auth, MemberId(id))?;
    match state.store.get_member_email(MemberId(id)).await? {
        Some(e) => Ok(Json(e)),
        None => Err(ApiError::NotFound),
    }
}

/// Clear a member's delivery email — opting out. Self-only; `404` if none was
/// set.
pub async fn delete_member_email(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
) -> ApiResult<StatusCode> {
    cap(&auth, WORKSPACE_READ)?;
    requested_member(&state, &auth, MemberId(id)).await?;
    ensure_own_personal_state(&auth, MemberId(id))?;
    if state.store.delete_member_email(MemberId(id)).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

/// Set a member's email delivery mode — `immediate` per-notification emails or
/// a periodic `digest`. Self-only for a session caller. An unknown mode is a
/// `400` (the enum fails deserialization at the extractor).
pub async fn set_member_delivery_mode(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
    ApiJson(body): ApiJson<SetDeliveryMode>,
) -> ApiResult<Json<DeliveryModeView>> {
    cap(&auth, WORKSPACE_READ)?;
    requested_member(&state, &auth, MemberId(id)).await?;
    ensure_own_personal_state(&auth, MemberId(id))?;
    state
        .store
        .set_delivery_mode(MemberId(id), body.mode)
        .await?;
    Ok(Json(DeliveryModeView { mode: body.mode }))
}

/// A member's email delivery mode — `immediate` when never set. Self-only for a
/// session caller.
pub async fn get_member_delivery_mode(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
) -> ApiResult<Json<DeliveryModeView>> {
    cap(&auth, WORKSPACE_READ)?;
    requested_member(&state, &auth, MemberId(id)).await?;
    ensure_own_personal_state(&auth, MemberId(id))?;
    let mode = state.store.get_delivery_mode(MemberId(id)).await?;
    Ok(Json(DeliveryModeView { mode }))
}

/// The waiting-on-you inbox: everything that needs this member's attention —
/// their assigned non-terminal threads, the reviews requested from them, the
/// reviews that name no reviewer and fall to them (owned by them, or ownerless
/// when they are a workspace admin), the workspace's pending approval gates,
/// and their unread mentions — oldest-waiting first, each aged against
/// `?sla_secs` (default 24h). `workspace:read` + self-only for a session (a
/// bearer orchestrator may query any member).
pub async fn get_member_waiting(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
    ApiQuery(q): ApiQuery<WaitingQuery>,
) -> ApiResult<Json<WaitingInbox>> {
    cap(&auth, WORKSPACE_READ)?;
    let member = requested_member(&state, &auth, MemberId(id)).await?;
    ensure_own_personal_state(&auth, MemberId(id))?;
    let sla = q.sla_secs.filter(|&s| s > 0).unwrap_or(86_400);
    let assigned = state
        .store
        .list_assigned_threads(member.workspace_id, MemberId(id))
        .await?;
    // Being named a reviewer does not grant access to a private channel: list
    // only the requests the caller can open (and so can act on).
    let mut reviews = Vec::new();
    for thread in state
        .store
        .list_review_requests(member.workspace_id, MemberId(id))
        .await?
    {
        if auth.bypass
            || maidan_auth::can_access_thread(state.store.as_ref(), &auth, thread.id).await?
        {
            reviews.push(thread);
        }
    }
    let unassigned = maidan_auth::visible_unassigned_reviews(
        state.store.as_ref(),
        &auth,
        member.workspace_id,
        MemberId(id),
    )
    .await?;
    let gates = state
        .store
        .list_pending_approval_gates(member.workspace_id, 200)
        .await?;
    let all_mentions = state
        .store
        .list_mentions_for_member(MemberId(id), 100)
        .await?;
    // Only mentions the member hasn't read yet are "waiting on you" (the inbox
    // cursor defaults to the epoch, so a never-read inbox keeps everything).
    let last_read = state.store.get_inbox_last_read_at(MemberId(id)).await?;
    let unread = all_mentions
        .into_iter()
        .filter(|m| m.created_at > last_read)
        .collect::<Vec<_>>();
    // Human/gate-blocked threads need owner or admin attention. A thread
    // reaches its owner; a thread with no owner reaches workspace admins.
    let mut blocked = Vec::new();
    {
        let now = chrono::Utc::now();
        let is_admin = auth.bypass
            || (auth.member_id == MemberId(id) && auth.has_capability(maidan_auth::TOKEN_ADMIN))
            || (auth.delegation_grant_id.is_none()
                && state
                    .store
                    .list_api_tokens_for_member(member.workspace_id, MemberId(id))
                    .await?
                    .iter()
                    .any(|t| {
                        t.revoked_at.is_none()
                            && t.expires_at.is_none_or(|at| at > now)
                            && t.capabilities.iter().any(|c| c == maidan_auth::TOKEN_ADMIN)
                    }));
        for (tid, title, owner, block) in state
            .store
            .list_human_gate_blocked_threads(member.workspace_id)
            .await?
        {
            let for_owner = owner == Some(MemberId(id));
            let for_admin = owner.is_none() && is_admin;
            if for_owner || for_admin {
                // Access-filter: don't show threads the caller can't open.
                if auth.bypass
                    || maidan_auth::can_access_thread(state.store.as_ref(), &auth, tid).await?
                {
                    blocked.push((tid, title, owner, block));
                }
            }
        }
    }
    Ok(Json(assemble_waiting_inbox(
        &assigned,
        &reviews,
        &unassigned,
        &gates,
        &unread,
        &blocked,
        chrono::Utc::now(),
        sla,
    )))
}

/// Register (upsert) a Web Push subscription for a member. Body is the
/// browser's `PushSubscription` JSON (`{endpoint, keys:{p256dh, auth}}`).
/// `workspace:read` + self-only for a session caller.
pub async fn register_push_subscription(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
    ApiJson(body): ApiJson<RegisterPushSubscription>,
) -> ApiResult<Json<PushSubscription>> {
    cap(&auth, WORKSPACE_READ)?;
    requested_member(&state, &auth, MemberId(id)).await?;
    ensure_own_personal_state(&auth, MemberId(id))?;
    if body.endpoint.trim().is_empty()
        || body.keys.p256dh.trim().is_empty()
        || body.keys.auth.trim().is_empty()
    {
        return Err(ApiError::BadRequest(
            "endpoint, keys.p256dh, and keys.auth are required".into(),
        ));
    }
    // The endpoint is where the server will POST, so it is an outbound target
    // like a webhook. Without this a member could point delivery at an
    // internal address and have the server call it on every notification.
    // Push services are HTTPS by definition (RFC 8030).
    let endpoint = maidan_auth::validate_egress_target(&body.endpoint)
        .map_err(|error| ApiError::BadRequest(format!("endpoint: {error}")))?;
    if endpoint.scheme() != "https" {
        return Err(ApiError::BadRequest("endpoint must be https".into()));
    }
    let sub = state
        .store
        .add_push_subscription(NewPushSubscription {
            member_id: MemberId(id),
            endpoint: body.endpoint,
            p256dh: body.keys.p256dh,
            auth: body.keys.auth,
        })
        .await?;
    Ok(Json(sub))
}

/// The VAPID public key the board passes to `PushManager.subscribe`.
/// `workspace:read`. When VAPID is not configured the body carries `reason`
/// `vapid_unset` and no key is invented.
pub async fn get_vapid_public_key(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
) -> ApiResult<Json<VapidPublicKey>> {
    cap(&auth, WORKSPACE_READ)?;
    if let Some(public_key) = state.web_push_public_key.clone() {
        return Ok(Json(VapidPublicKey {
            public_key: Some(public_key),
            reason: None,
        }));
    }
    Ok(Json(VapidPublicKey {
        public_key: None,
        reason: state
            .web_push_disabled_reason
            .clone()
            .or_else(|| Some("vapid_unset".into())),
    }))
}

/// A member Web Push subscriptions. Self-only for a session.
pub async fn list_push_subscriptions(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
) -> ApiResult<Json<Vec<PushSubscription>>> {
    cap(&auth, WORKSPACE_READ)?;
    requested_member(&state, &auth, MemberId(id)).await?;
    ensure_own_personal_state(&auth, MemberId(id))?;
    Ok(Json(
        state.store.list_push_subscriptions(MemberId(id)).await?,
    ))
}

/// Remove one of a member's Web Push subscriptions — recipient- scoped in the
/// store (`404` when it isn't this member's). Self-only for a session.
pub async fn delete_push_subscription(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath((id, sub_id)): ApiPath<(uuid::Uuid, uuid::Uuid)>,
) -> ApiResult<StatusCode> {
    cap(&auth, WORKSPACE_READ)?;
    requested_member(&state, &auth, MemberId(id)).await?;
    ensure_own_personal_state(&auth, MemberId(id))?;
    if state
        .store
        .delete_push_subscription(MemberId(id), PushSubscriptionId(sub_id))
        .await?
    {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

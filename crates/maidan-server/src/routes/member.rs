//! Member handlers: list/get members, mentions-for-member, and inbox.

use axum::http::StatusCode;
use axum::{
    extract::{Path, Query, State},
    Extension, Json,
};
use maidan_auth::{capability::WORKSPACE_READ, AuthContext};
use maidan_types::*;

use super::{cap, ensure_acting_member, ensure_workspace, ApiResult};
use crate::dto::*;
use crate::error::{ApiError, ApiJson};
use crate::state::AppState;

/// `GET /me` — the caller's own identity (Cluster 337; the REST twin of the MCP
/// `whoami` tool). Reflects the request's auth (member/workspace/capabilities), so
/// an agent handed only a base URL + token can discover the `member_id` every
/// member-attributed write needs. `workspace:read`.
pub async fn get_me(Extension(auth): Extension<AuthContext>) -> ApiResult<Json<WhoAmI>> {
    cap(&auth, WORKSPACE_READ)?;
    Ok(Json(WhoAmI {
        member_id: auth.member_id.0,
        workspace_id: auth.workspace_id.0,
        capabilities: auth.capabilities().to_vec(),
        is_bearer: auth.token_id.is_some(),
        known_capabilities: maidan_auth::capability::all(),
    }))
}

#[cfg(feature = "bootstrap")]
pub async fn create_member(
    State(state): State<AppState>,
    Path(workspace_id): Path<uuid::Uuid>,
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
    Path(workspace_id): Path<uuid::Uuid>,
) -> ApiResult<Json<Vec<Member>>> {
    let workspace_id = WorkspaceId(workspace_id);
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, workspace_id)?;
    Ok(Json(state.store.list_members(workspace_id).await?))
}

pub async fn get_member(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<Member>> {
    let member = state.store.get_member(MemberId(id)).await?;
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, member.workspace_id)?;
    Ok(Json(member))
}

pub async fn list_mentions_for_member(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
    Query(q): Query<ListMentionsQuery>,
) -> ApiResult<Json<Vec<Mention>>> {
    let member = state.store.get_member(MemberId(id)).await?;
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, member.workspace_id)?;
    // Defensive self-only (Cluster 315). These legacy inbox/mentions routes are mounted
    // ONLY on the bearer-only `protected` router, and a bearer is the act-as-any
    // orchestrator (Cluster-202/203 model), so this is a strict no-op for every current
    // caller. It matches the sibling notification handlers and pins a session to self IF
    // these are ever also session-mounted under `/ui/api` (the Cluster-251 pattern).
    ensure_acting_member(&auth, MemberId(id))?;
    Ok(Json(
        state
            .store
            .list_mentions_for_member(MemberId(id), q.limit)
            .await?,
    ))
}

pub async fn get_member_inbox(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
    Query(q): Query<ListInboxQuery>,
) -> ApiResult<Json<MemberInbox>> {
    let member = state.store.get_member(MemberId(id)).await?;
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, member.workspace_id)?;
    // Defensive self-only (Cluster 315; bearer-only route today → no-op for current
    // callers; guards a future `/ui/api` session mount). See list_mentions_for_member.
    ensure_acting_member(&auth, MemberId(id))?;
    Ok(Json(
        state.store.list_member_inbox(MemberId(id), q.limit).await?,
    ))
}

pub async fn mark_member_inbox_read(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<MarkInboxRead>,
) -> ApiResult<Json<MemberInbox>> {
    let member = state.store.get_member(MemberId(id)).await?;
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, member.workspace_id)?;
    // Defensive self-only (Cluster 315). `advance_inbox_last_read_at` is a per-member
    // write; bearer-only route today (act-as-any → no-op), guards a future session mount.
    ensure_acting_member(&auth, MemberId(id))?;
    state
        .store
        .advance_inbox_last_read_at(MemberId(id), body.read_through)
        .await?;
    Ok(Json(state.store.list_member_inbox(MemberId(id), 50).await?))
}

/// A member's per-recipient notifications, newest first (Cluster 239). Self-only for
/// a session caller (a member reads their OWN inbox); a bearer is the act-as-any
/// orchestrator (the Cluster-202/203 model).
pub async fn list_member_notifications(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
    Query(q): Query<ListNotificationsQuery>,
) -> ApiResult<Json<Vec<Notification>>> {
    cap(&auth, WORKSPACE_READ)?;
    let member = state.store.get_member(MemberId(id)).await?;
    ensure_workspace(&auth, member.workspace_id)?;
    ensure_acting_member(&auth, MemberId(id))?;
    let limit = q.limit.clamp(1, 500);
    Ok(Json(
        state
            .store
            .list_notifications(MemberId(id), q.unread_only, limit)
            .await?,
    ))
}

/// A member's notifications collapsed into per-thread groups, newest-activity
/// first (Cluster 359, N5) — so a busy thread shows as one row instead of
/// flooding the flat inbox. `limit` bounds how many notifications are scanned
/// (default 200, clamp 1..=500); `unread_only` groups only unread ones. Self-only
/// for a session caller.
pub async fn list_member_notifications_grouped(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
    Query(q): Query<ListNotificationsQuery>,
) -> ApiResult<Json<Vec<NotificationThreadGroup>>> {
    cap(&auth, WORKSPACE_READ)?;
    let member = state.store.get_member(MemberId(id)).await?;
    ensure_workspace(&auth, member.workspace_id)?;
    ensure_acting_member(&auth, MemberId(id))?;
    let limit = q.limit.clamp(1, 500);
    let notes = state
        .store
        .list_notifications(MemberId(id), q.unread_only, limit)
        .await?;
    Ok(Json(maidan_types::group_notifications_by_thread(&notes)))
}

/// A member's buried decisions (Cluster 359, N2) — task results produced by
/// someone else in a channel/thread the member follows, since `since` (default
/// 7 days ago), newest first. The queryable form of the buried-decisions digest.
/// Self-only for a session caller; `workspace:read`.
pub async fn list_member_decisions(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
    Query(q): Query<DecisionsQuery>,
) -> ApiResult<Json<Vec<BuriedDecision>>> {
    cap(&auth, WORKSPACE_READ)?;
    let member = state.store.get_member(MemberId(id)).await?;
    ensure_workspace(&auth, member.workspace_id)?;
    ensure_acting_member(&auth, MemberId(id))?;
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

/// A member's unread-notification badge count (Cluster 239). Self-only for sessions.
pub async fn member_unread_notification_count(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<UnreadCount>> {
    cap(&auth, WORKSPACE_READ)?;
    let member = state.store.get_member(MemberId(id)).await?;
    ensure_workspace(&auth, member.workspace_id)?;
    ensure_acting_member(&auth, MemberId(id))?;
    let count = state.store.unread_notification_count(MemberId(id)).await?;
    Ok(Json(UnreadCount { count }))
}

/// Mark one of a member's notifications read (Cluster 239). Self-only for sessions;
/// the store scopes the write to `(member_id, id)`, so `404` when the notification
/// isn't this member's. Returns the new unread count.
pub async fn mark_member_notification_read(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path((id, nid)): Path<(uuid::Uuid, uuid::Uuid)>,
) -> ApiResult<Json<UnreadCount>> {
    cap(&auth, WORKSPACE_READ)?;
    let member = state.store.get_member(MemberId(id)).await?;
    ensure_workspace(&auth, member.workspace_id)?;
    ensure_acting_member(&auth, MemberId(id))?;
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

/// Snooze one notification until `until` (Cluster 359, N5) — it drops out of the
/// inbox + badge until then. Self-only for a session caller; `404` when the
/// notification isn't this member's. Returns the new unread count.
pub async fn snooze_member_notification(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path((id, nid)): Path<(uuid::Uuid, uuid::Uuid)>,
    ApiJson(body): ApiJson<SnoozeNotification>,
) -> ApiResult<Json<UnreadCount>> {
    cap(&auth, WORKSPACE_READ)?;
    let member = state.store.get_member(MemberId(id)).await?;
    ensure_workspace(&auth, member.workspace_id)?;
    ensure_acting_member(&auth, MemberId(id))?;
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

/// Mark all of a member's notifications read (Cluster 239). Self-only for sessions.
pub async fn mark_all_member_notifications_read(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<MarkAllRead>> {
    cap(&auth, WORKSPACE_READ)?;
    let member = state.store.get_member(MemberId(id)).await?;
    ensure_workspace(&auth, member.workspace_id)?;
    ensure_acting_member(&auth, MemberId(id))?;
    let cleared = state
        .store
        .mark_all_notifications_read(MemberId(id))
        .await? as i64;
    Ok(Json(MarkAllRead { cleared }))
}

/// Set a member's mute preference for an event kind (Cluster 242). Self-only for a
/// session caller (a member configures their OWN preferences); a bearer is the
/// act-as-any orchestrator.
pub async fn set_member_notification_pref(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<SetNotificationPref>,
) -> ApiResult<Json<NotificationPref>> {
    cap(&auth, WORKSPACE_READ)?;
    let member = state.store.get_member(MemberId(id)).await?;
    ensure_workspace(&auth, member.workspace_id)?;
    ensure_acting_member(&auth, MemberId(id))?;
    Ok(Json(
        state
            .store
            .set_notification_pref(MemberId(id), body.kind, body.muted)
            .await?,
    ))
}

/// A member's notification preferences (Cluster 242). Self-only for sessions.
pub async fn list_member_notification_prefs(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<Vec<NotificationPref>>> {
    cap(&auth, WORKSPACE_READ)?;
    let member = state.store.get_member(MemberId(id)).await?;
    ensure_workspace(&auth, member.workspace_id)?;
    ensure_acting_member(&auth, MemberId(id))?;
    Ok(Json(
        state.store.list_notification_prefs(MemberId(id)).await?,
    ))
}

/// Follow a channel to be notified of activity there (Cluster 245). Self-only for a
/// session caller; requires access to the channel being followed.
pub async fn follow_member_channel(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<FollowChannel>,
) -> ApiResult<StatusCode> {
    cap(&auth, WORKSPACE_READ)?;
    let member = state.store.get_member(MemberId(id)).await?;
    ensure_workspace(&auth, member.workspace_id)?;
    ensure_acting_member(&auth, MemberId(id))?;
    maidan_auth::ensure_channel_access(state.store.as_ref(), &auth, body.channel_id).await?;
    state
        .store
        .follow_channel(MemberId(id), body.channel_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Unfollow a channel (Cluster 245). Self-only for sessions; `404` if not following.
pub async fn unfollow_member_channel(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path((id, cid)): Path<(uuid::Uuid, uuid::Uuid)>,
) -> ApiResult<StatusCode> {
    cap(&auth, WORKSPACE_READ)?;
    let member = state.store.get_member(MemberId(id)).await?;
    ensure_workspace(&auth, member.workspace_id)?;
    ensure_acting_member(&auth, MemberId(id))?;
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

/// The channels a member follows (Cluster 245). Self-only for sessions.
pub async fn list_member_channel_follows(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<Vec<ChannelFollow>>> {
    cap(&auth, WORKSPACE_READ)?;
    let member = state.store.get_member(MemberId(id)).await?;
    ensure_workspace(&auth, member.workspace_id)?;
    ensure_acting_member(&auth, MemberId(id))?;
    Ok(Json(state.store.list_channel_follows(MemberId(id)).await?))
}

/// Follow a thread to be notified of activity there (Cluster 245). Self-only for a
/// session caller; requires access to the thread being followed.
pub async fn follow_member_thread(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<FollowThread>,
) -> ApiResult<StatusCode> {
    cap(&auth, WORKSPACE_READ)?;
    let member = state.store.get_member(MemberId(id)).await?;
    ensure_workspace(&auth, member.workspace_id)?;
    ensure_acting_member(&auth, MemberId(id))?;
    maidan_auth::ensure_thread_access(state.store.as_ref(), &auth, body.thread_id).await?;
    state
        .store
        .follow_thread(MemberId(id), body.thread_id)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Unfollow a thread (Cluster 245). Self-only for sessions; `404` if not following.
pub async fn unfollow_member_thread(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path((id, tid)): Path<(uuid::Uuid, uuid::Uuid)>,
) -> ApiResult<StatusCode> {
    cap(&auth, WORKSPACE_READ)?;
    let member = state.store.get_member(MemberId(id)).await?;
    ensure_workspace(&auth, member.workspace_id)?;
    ensure_acting_member(&auth, MemberId(id))?;
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

/// The threads a member follows (Cluster 245). Self-only for sessions.
pub async fn list_member_thread_follows(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<Vec<ThreadFollow>>> {
    cap(&auth, WORKSPACE_READ)?;
    let member = state.store.get_member(MemberId(id)).await?;
    ensure_workspace(&auth, member.workspace_id)?;
    ensure_acting_member(&auth, MemberId(id))?;
    Ok(Json(state.store.list_thread_follows(MemberId(id)).await?))
}

/// Set a member's delivery email address (Cluster 250) — opting in to email
/// notifications. Self-only for a session caller. A light `@` sanity check rejects
/// obvious garbage; the transport validates fully on send.
pub async fn set_member_email(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<SetEmail>,
) -> ApiResult<Json<MemberEmail>> {
    cap(&auth, WORKSPACE_READ)?;
    let member = state.store.get_member(MemberId(id)).await?;
    ensure_workspace(&auth, member.workspace_id)?;
    ensure_acting_member(&auth, MemberId(id))?;
    let email = body.email.trim();
    if !email.contains('@') || email.len() < 3 {
        return Err(ApiError::BadRequest("email must be a valid address".into()));
    }
    Ok(Json(
        state.store.set_member_email(MemberId(id), email).await?,
    ))
}

/// A member's delivery email, or `404` if unset (Cluster 250). Self-only for sessions.
pub async fn get_member_email(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<MemberEmail>> {
    cap(&auth, WORKSPACE_READ)?;
    let member = state.store.get_member(MemberId(id)).await?;
    ensure_workspace(&auth, member.workspace_id)?;
    ensure_acting_member(&auth, MemberId(id))?;
    match state.store.get_member_email(MemberId(id)).await? {
        Some(e) => Ok(Json(e)),
        None => Err(ApiError::NotFound),
    }
}

/// Clear a member's delivery email (Cluster 250) — opting out. Self-only; `404` if
/// none was set.
pub async fn delete_member_email(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<StatusCode> {
    cap(&auth, WORKSPACE_READ)?;
    let member = state.store.get_member(MemberId(id)).await?;
    ensure_workspace(&auth, member.workspace_id)?;
    ensure_acting_member(&auth, MemberId(id))?;
    if state.store.delete_member_email(MemberId(id)).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

/// Set a member's email delivery mode (Cluster 256) — `immediate` per-notification
/// emails or a periodic `digest`. Self-only for a session caller. An unknown mode
/// is a `400` (the enum fails deserialization at the extractor).
pub async fn set_member_delivery_mode(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<SetDeliveryMode>,
) -> ApiResult<Json<DeliveryModeView>> {
    cap(&auth, WORKSPACE_READ)?;
    let member = state.store.get_member(MemberId(id)).await?;
    ensure_workspace(&auth, member.workspace_id)?;
    ensure_acting_member(&auth, MemberId(id))?;
    state
        .store
        .set_delivery_mode(MemberId(id), body.mode)
        .await?;
    Ok(Json(DeliveryModeView { mode: body.mode }))
}

/// A member's email delivery mode (Cluster 256) — `immediate` when never set.
/// Self-only for a session caller.
pub async fn get_member_delivery_mode(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<DeliveryModeView>> {
    cap(&auth, WORKSPACE_READ)?;
    let member = state.store.get_member(MemberId(id)).await?;
    ensure_workspace(&auth, member.workspace_id)?;
    ensure_acting_member(&auth, MemberId(id))?;
    let mode = state.store.get_delivery_mode(MemberId(id)).await?;
    Ok(Json(DeliveryModeView { mode }))
}

/// The waiting-on-you inbox (Cluster 368, Wave 2 #16 — G15/G9): everything that
/// needs this member's attention — their assigned non-terminal threads, the
/// workspace's pending approval gates, and their unread mentions — oldest-waiting
/// first, each aged against `?sla_secs` (default 24h). `workspace:read` + self-only
/// for a session (a bearer orchestrator may query any member).
pub async fn get_member_waiting(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
    Query(q): Query<WaitingQuery>,
) -> ApiResult<Json<WaitingInbox>> {
    cap(&auth, WORKSPACE_READ)?;
    let member = state.store.get_member(MemberId(id)).await?;
    ensure_workspace(&auth, member.workspace_id)?;
    ensure_acting_member(&auth, MemberId(id))?;
    let sla = q.sla_secs.filter(|&s| s > 0).unwrap_or(86_400);
    let assigned = state
        .store
        .list_assigned_threads(member.workspace_id, MemberId(id))
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
    Ok(Json(assemble_waiting_inbox(
        &assigned,
        &gates,
        &unread,
        chrono::Utc::now(),
        sla,
    )))
}

/// Register (upsert) a Web Push subscription for a member (Cluster 366, N1). Body
/// is the browser's `PushSubscription` JSON (`{endpoint, keys:{p256dh, auth}}`).
/// `workspace:read` + self-only for a session caller.
pub async fn register_push_subscription(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<RegisterPushSubscription>,
) -> ApiResult<Json<PushSubscription>> {
    cap(&auth, WORKSPACE_READ)?;
    let member = state.store.get_member(MemberId(id)).await?;
    ensure_workspace(&auth, member.workspace_id)?;
    ensure_acting_member(&auth, MemberId(id))?;
    if body.endpoint.trim().is_empty()
        || body.keys.p256dh.trim().is_empty()
        || body.keys.auth.trim().is_empty()
    {
        return Err(ApiError::BadRequest(
            "endpoint, keys.p256dh, and keys.auth are required".into(),
        ));
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

/// A member's Web Push subscriptions (Cluster 366, N1). Self-only for a session.
pub async fn list_push_subscriptions(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<Vec<PushSubscription>>> {
    cap(&auth, WORKSPACE_READ)?;
    let member = state.store.get_member(MemberId(id)).await?;
    ensure_workspace(&auth, member.workspace_id)?;
    ensure_acting_member(&auth, MemberId(id))?;
    Ok(Json(
        state.store.list_push_subscriptions(MemberId(id)).await?,
    ))
}

/// Remove one of a member's Web Push subscriptions (Cluster 366, N1) — recipient-
/// scoped in the store (`404` when it isn't this member's). Self-only for a session.
pub async fn delete_push_subscription(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path((id, sub_id)): Path<(uuid::Uuid, uuid::Uuid)>,
) -> ApiResult<StatusCode> {
    cap(&auth, WORKSPACE_READ)?;
    let member = state.store.get_member(MemberId(id)).await?;
    ensure_workspace(&auth, member.workspace_id)?;
    ensure_acting_member(&auth, MemberId(id))?;
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

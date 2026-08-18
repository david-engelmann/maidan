//! Member handlers: list/get members, mentions-for-member, and inbox.

#[cfg(feature = "bootstrap")]
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

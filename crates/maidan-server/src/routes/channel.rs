//! Channel handlers: create/list/get channels.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use chrono::Utc;
use maidan_auth::{
    capability::{CHANNEL_ADMIN, WORKSPACE_READ, WORKSPACE_WRITE},
    AuthContext,
};
use maidan_types::*;

use super::{cap, ensure_workspace, publish, ApiResult};
use crate::dto::*;
use crate::error::ApiJson;
use crate::state::AppState;

pub async fn create_channel(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(workspace_id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<CreateChannel>,
) -> ApiResult<(StatusCode, Json<Channel>)> {
    let workspace_id = WorkspaceId(workspace_id);
    cap(&auth, WORKSPACE_WRITE)?;
    ensure_workspace(&auth, workspace_id)?;
    let c = state
        .store
        .create_channel(NewChannel {
            workspace_id,
            name: body.name,
            topic: body.topic,
            private: body.private,
        })
        .await?;
    // Auto-add the creator as an admin of a new private channel so they don't
    // lock themselves out (Cluster 160). Bypass callers have no real member.
    if c.private && !auth.bypass {
        state
            .store
            .add_channel_member(c.id, auth.member_id, ChannelMemberRole::Admin)
            .await?;
    }
    publish(
        &state,
        Event::ChannelCreated {
            occurred_at: Utc::now(),
            workspace_id,
            channel: c.clone(),
        },
    )
    .await;
    Ok((StatusCode::CREATED, Json(c)))
}

pub async fn list_channels(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(workspace_id): Path<uuid::Uuid>,
) -> ApiResult<Json<Vec<Channel>>> {
    let workspace_id = WorkspaceId(workspace_id);
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, workspace_id)?;
    let all = state.store.list_channels(workspace_id).await?;
    if auth.bypass {
        return Ok(Json(all));
    }
    // Hide private channels the caller is not a member of (Cluster 160). Public
    // and the DM system channel are always listed.
    let mut visible = Vec::with_capacity(all.len());
    for ch in all {
        if !ch.private
            || ch.name == DM_CHANNEL_NAME
            || state.store.channel_is_member(ch.id, auth.member_id).await?
        {
            visible.push(ch);
        }
    }
    Ok(Json(visible))
}

pub async fn get_channel(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<Channel>> {
    let channel = state.store.get_channel(ChannelId(id)).await?;
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, channel.workspace_id)?;
    maidan_auth::ensure_channel_access(state.store.as_ref(), &auth, channel.id).await?;
    Ok(Json(channel))
}

pub async fn add_channel_member(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(cid): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<AddChannelMember>,
) -> ApiResult<(StatusCode, Json<ChannelMember>)> {
    let channel = state.store.get_channel(ChannelId(cid)).await?;
    cap(&auth, CHANNEL_ADMIN)?;
    ensure_workspace(&auth, channel.workspace_id)?;
    let role = body.role.unwrap_or(ChannelMemberRole::Member);
    let m = state
        .store
        .add_channel_member(channel.id, MemberId(body.member_id), role)
        .await?;
    Ok((StatusCode::CREATED, Json(m)))
}

pub async fn list_channel_members(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(cid): Path<uuid::Uuid>,
) -> ApiResult<Json<Vec<ChannelMember>>> {
    let channel = state.store.get_channel(ChannelId(cid)).await?;
    cap(&auth, CHANNEL_ADMIN)?;
    ensure_workspace(&auth, channel.workspace_id)?;
    Ok(Json(state.store.list_channel_members(channel.id).await?))
}

pub async fn remove_channel_member(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path((cid, mid)): Path<(uuid::Uuid, uuid::Uuid)>,
) -> ApiResult<StatusCode> {
    let channel = state.store.get_channel(ChannelId(cid)).await?;
    cap(&auth, CHANNEL_ADMIN)?;
    ensure_workspace(&auth, channel.workspace_id)?;
    state
        .store
        .remove_channel_member(channel.id, MemberId(mid))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

//! Channel handlers: create/list/get channels.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use chrono::Utc;
use maidan_auth::{
    capability::{WORKSPACE_READ, WORKSPACE_WRITE},
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
    Ok(Json(state.store.list_channels(workspace_id).await?))
}

pub async fn get_channel(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<Channel>> {
    let channel = state.store.get_channel(ChannelId(id)).await?;
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, channel.workspace_id)?;
    Ok(Json(channel))
}

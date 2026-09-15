//! Room discovery, handle aliases, and the authenticated room card
//! (Cluster 395, Wave 3 #35 B22).
//!
//! `GET /.well-known/maidan-room` is public and scheme-only — no tenant
//! list. Handle writes rename the alias; the workspace UUID (and every
//! stored `maidan://{uuid}/…` URI) stays put.

use axum::{
    extract::{Path, State},
    Extension, Json,
};
use maidan_auth::{
    capability::{WORKSPACE_READ, WORKSPACE_WRITE},
    AuthContext,
};
use maidan_types::{RoomCard, RoomDiscovery, WorkspaceHandle, WorkspaceId};

use super::{cap, ensure_workspace, ApiResult};
use crate::dto::SetWorkspaceHandle;
use crate::error::{ApiError, ApiJson};
use crate::state::AppState;

/// Public discovery document. No auth — and no workspace list.
pub async fn well_known_room() -> Json<RoomDiscovery> {
    Json(RoomDiscovery::document())
}

pub async fn get_workspace_room(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<RoomCard>> {
    let workspace_id = WorkspaceId(id);
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, workspace_id)?;
    let _ = state.store.get_workspace(workspace_id).await?;
    let handle = state
        .store
        .get_workspace_handle(workspace_id)
        .await?
        .map(|h| h.handle);
    Ok(Json(RoomCard::new(workspace_id, handle)))
}

pub async fn get_workspace_handle(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<WorkspaceHandle>> {
    let workspace_id = WorkspaceId(id);
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, workspace_id)?;
    state
        .store
        .get_workspace_handle(workspace_id)
        .await?
        .map(Json)
        .ok_or(ApiError::NotFound)
}

pub async fn set_workspace_handle(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<SetWorkspaceHandle>,
) -> ApiResult<Json<WorkspaceHandle>> {
    let workspace_id = WorkspaceId(id);
    cap(&auth, WORKSPACE_WRITE)?;
    ensure_workspace(&auth, workspace_id)?;
    Ok(Json(
        state
            .store
            .set_workspace_handle(workspace_id, &body.handle)
            .await?,
    ))
}

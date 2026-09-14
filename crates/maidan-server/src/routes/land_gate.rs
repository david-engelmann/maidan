//! Land-gate pointer (Cluster 385.3, renamed Cluster 389). The room
//! holds `{kind:"land_gate", status:pass|fail, artifact_sha?}` plus the
//! green/amber/red land vocabulary. An external verifier records
//! pass/fail. Writes are `thread:transition`; reads are `workspace:read`.
//! The FSM close-gate (385.2) enforces a qualifying green pass.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use maidan_auth::{
    capability::{THREAD_TRANSITION, WORKSPACE_READ},
    AuthContext,
};
use maidan_types::*;

use super::{cap, ApiResult};
use crate::dto::SetLandGate;
use crate::error::{ApiError, ApiJson};
use crate::state::AppState;

pub async fn set_land_gate(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<SetLandGate>,
) -> ApiResult<Json<LandGateStanding>> {
    cap(&auth, THREAD_TRANSITION)?;
    let thread_id = ThreadId(id);
    maidan_auth::authorize_thread(state.store.as_ref(), &auth, thread_id).await?;
    let sha = body
        .artifact_sha
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let standing = state
        .store
        .set_land_gate_pointer(thread_id, auth.member_id, body.status, sha, body.land)
        .await?;
    Ok(Json(standing))
}

pub async fn get_land_gate(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<LandGateStanding>> {
    cap(&auth, WORKSPACE_READ)?;
    let thread_id = ThreadId(id);
    maidan_auth::authorize_thread(state.store.as_ref(), &auth, thread_id).await?;
    Ok(Json(state.store.get_land_gate_standing(thread_id).await?))
}

pub async fn clear_land_gate(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<StatusCode> {
    cap(&auth, THREAD_TRANSITION)?;
    let thread_id = ThreadId(id);
    maidan_auth::authorize_thread(state.store.as_ref(), &auth, thread_id).await?;
    if state.store.clear_land_gate(thread_id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

pub async fn require_land_gate(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<LandGateStanding>> {
    cap(&auth, THREAD_TRANSITION)?;
    let thread_id = ThreadId(id);
    maidan_auth::authorize_thread(state.store.as_ref(), &auth, thread_id).await?;
    Ok(Json(state.store.require_land_gate(thread_id).await?))
}

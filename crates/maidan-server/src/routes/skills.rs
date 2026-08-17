//! Capability-registry management (Cluster 232, Arc E): declare / list / remove a
//! member's skills, and set / list / remove a task's required skills. Skill
//! routing (Cluster 231) reads both to gate `claim_next`.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use maidan_auth::{
    capability::{THREAD_TRANSITION, WORKSPACE_READ, WORKSPACE_WRITE},
    AuthContext,
};
use maidan_router::resolve_thread_context;
use maidan_types::*;

use super::{cap, ensure_workspace, ApiResult};
use crate::dto::*;
use crate::error::{ApiError, ApiJson};
use crate::state::AppState;

// --- member skills ---

pub async fn add_member_skill(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<AddSkill>,
) -> ApiResult<StatusCode> {
    let member = state.store.get_member(MemberId(id)).await?;
    cap(&auth, WORKSPACE_WRITE)?;
    ensure_workspace(&auth, member.workspace_id)?;
    if body.skill.trim().is_empty() {
        return Err(ApiError::BadRequest("skill must not be empty".into()));
    }
    state
        .store
        .add_member_skill(member.id, body.skill.trim())
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_member_skills(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<Vec<MemberSkill>>> {
    let member = state.store.get_member(MemberId(id)).await?;
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, member.workspace_id)?;
    Ok(Json(state.store.list_member_skills(member.id).await?))
}

pub async fn remove_member_skill(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path((id, skill)): Path<(uuid::Uuid, String)>,
) -> ApiResult<StatusCode> {
    let member = state.store.get_member(MemberId(id)).await?;
    cap(&auth, WORKSPACE_WRITE)?;
    ensure_workspace(&auth, member.workspace_id)?;
    if state.store.remove_member_skill(member.id, &skill).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

// --- thread required skills ---

pub async fn add_thread_required_skill(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<AddSkill>,
) -> ApiResult<StatusCode> {
    let thread_id = ThreadId(id);
    let ctx = resolve_thread_context(state.store.as_ref(), thread_id).await?;
    cap(&auth, THREAD_TRANSITION)?;
    ensure_workspace(&auth, ctx.workspace_id)?;
    maidan_auth::ensure_thread_access(state.store.as_ref(), &auth, thread_id).await?;
    if body.skill.trim().is_empty() {
        return Err(ApiError::BadRequest("skill must not be empty".into()));
    }
    state
        .store
        .add_thread_required_skill(thread_id, body.skill.trim())
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_thread_required_skills(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<Vec<ThreadRequiredSkill>>> {
    let thread_id = ThreadId(id);
    let ctx = resolve_thread_context(state.store.as_ref(), thread_id).await?;
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, ctx.workspace_id)?;
    maidan_auth::ensure_thread_access(state.store.as_ref(), &auth, thread_id).await?;
    Ok(Json(
        state.store.list_thread_required_skills(thread_id).await?,
    ))
}

pub async fn remove_thread_required_skill(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path((id, skill)): Path<(uuid::Uuid, String)>,
) -> ApiResult<StatusCode> {
    let thread_id = ThreadId(id);
    let ctx = resolve_thread_context(state.store.as_ref(), thread_id).await?;
    cap(&auth, THREAD_TRANSITION)?;
    ensure_workspace(&auth, ctx.workspace_id)?;
    maidan_auth::ensure_thread_access(state.store.as_ref(), &auth, thread_id).await?;
    if state
        .store
        .remove_thread_required_skill(thread_id, &skill)
        .await?
    {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

//! Member-freeze kill-switch management (Cluster 372.3, Wave 2 #20, G17/B25). An
//! operator freezes a member (`token:admin`) — which drops their leases and makes
//! `claim_next` refuse them — and unfreezes to lift it. Freeze/unfreeze are
//! audited (a security-sensitive mutation). Not G4 PAUSE.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use maidan_auth::{capability::TOKEN_ADMIN, AuthContext};
use maidan_types::*;

use super::{cap, ensure_workspace, ApiResult};
use crate::dto::*;
use crate::error::{ApiError, ApiJson};
use crate::state::AppState;

/// Resolve the target member and authorize the caller for their workspace.
async fn authorize_member(
    state: &AppState,
    auth: &AuthContext,
    member_id: MemberId,
) -> ApiResult<Member> {
    let member = state.store.get_member(member_id).await?;
    ensure_workspace(auth, member.workspace_id)?;
    Ok(member)
}

pub async fn freeze_member(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<FreezeMember>,
) -> ApiResult<Json<FreezeResult>> {
    cap(&auth, TOKEN_ADMIN)?;
    let member_id = MemberId(id);
    authorize_member(&state, &auth, member_id).await?;
    let reason = body
        .reason
        .as_deref()
        .map(str::trim)
        .filter(|r| !r.is_empty());
    let (freeze, released) = state
        .store
        .freeze_member(member_id, auth.member_id, reason)
        .await?;
    crate::audit::record(
        &state,
        NewAuditEvent {
            actor_id: Some(auth.member_id),
            action: "member.freeze".into(),
            target_kind: Some("member".into()),
            target_id: Some(member_id.0),
            metadata: serde_json::json!({ "reason": reason, "released": released }),
        },
    )
    .await;
    Ok(Json(FreezeResult { freeze, released }))
}

pub async fn unfreeze_member(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<StatusCode> {
    cap(&auth, TOKEN_ADMIN)?;
    let member_id = MemberId(id);
    authorize_member(&state, &auth, member_id).await?;
    if !state.store.unfreeze_member(member_id).await? {
        return Err(ApiError::NotFound);
    }
    crate::audit::record(
        &state,
        NewAuditEvent {
            actor_id: Some(auth.member_id),
            action: "member.unfreeze".into(),
            target_kind: Some("member".into()),
            target_id: Some(member_id.0),
            metadata: serde_json::json!({}),
        },
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn get_member_freeze(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<MemberFreeze>> {
    cap(&auth, TOKEN_ADMIN)?;
    let member_id = MemberId(id);
    authorize_member(&state, &auth, member_id).await?;
    state
        .store
        .get_member_freeze(member_id)
        .await?
        .map(Json)
        .ok_or(ApiError::NotFound)
}

pub async fn list_frozen_members(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(workspace_id): Path<uuid::Uuid>,
) -> ApiResult<Json<Vec<MemberFreeze>>> {
    let workspace_id = WorkspaceId(workspace_id);
    cap(&auth, TOKEN_ADMIN)?;
    ensure_workspace(&auth, workspace_id)?;
    Ok(Json(state.store.list_frozen_members(workspace_id).await?))
}

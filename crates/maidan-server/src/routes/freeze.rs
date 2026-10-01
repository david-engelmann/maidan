//! Member-freeze kill-switch management. An operator freezes a member
//! (`token:admin`) — which drops their leases and makes `claim_next` refuse
//! them — and unfreezes to lift it. Freeze/unfreeze are audited (a
//! security-sensitive mutation) and announced as `MemberFrozen` /
//! `MemberUnfrozen`. Not G4 PAUSE.

use axum::{extract::State, http::StatusCode, Extension, Json};
use maidan_auth::{capability::TOKEN_ADMIN, AuthContext};
use maidan_types::*;

use super::{cap, ensure_workspace, publish_stored, requested_member, ApiResult};
use crate::dto::*;
use crate::error::ApiError;
use crate::extract::{ApiJson, ApiPath};
use crate::state::AppState;

/// Resolve the target member and authorize the caller for their workspace.
async fn authorize_member(
    state: &AppState,
    auth: &AuthContext,
    member_id: MemberId,
) -> ApiResult<Member> {
    requested_member(state, auth, member_id).await
}

pub async fn freeze_member(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
    ApiJson(body): ApiJson<FreezeMember>,
) -> ApiResult<Json<FreezeResult>> {
    cap(&auth, TOKEN_ADMIN)?;
    let member_id = MemberId(id);
    let workspace_id = authorize_member(&state, &auth, member_id)
        .await?
        .workspace_id;
    let reason = body
        .reason
        .as_deref()
        .map(str::trim)
        .filter(|r| !r.is_empty());
    let actor = auth.actor_id;
    let (freeze, released, stored) = state
        .store
        .freeze_member_audited(
            member_id,
            auth.member_id,
            reason,
            Box::new(move |(freeze, released)| NewAuditEvent {
                scope: AuditScope::Workspace(workspace_id),
                actor_id: Some(actor),
                action: "member.freeze".into(),
                target_kind: Some("member".into()),
                target_id: Some(member_id.0),
                metadata: serde_json::json!({ "reason": freeze.reason, "released": released }),
            }),
        )
        .await?;
    publish_stored(&state, stored).await;
    Ok(Json(FreezeResult { freeze, released }))
}

pub async fn unfreeze_member(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
) -> ApiResult<StatusCode> {
    cap(&auth, TOKEN_ADMIN)?;
    let member_id = MemberId(id);
    let member = authorize_member(&state, &auth, member_id).await?;
    let stored = state
        .store
        .unfreeze_member_audited(
            member_id,
            auth.member_id,
            NewAuditEvent {
                scope: AuditScope::Workspace(member.workspace_id),
                actor_id: Some(auth.actor_id),
                action: "member.unfreeze".into(),
                target_kind: Some("member".into()),
                target_id: Some(member_id.0),
                metadata: serde_json::json!({}),
            },
        )
        .await?;
    let Some(stored) = stored else {
        return Err(ApiError::NotFound);
    };
    publish_stored(&state, stored).await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn get_member_freeze(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
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
    ApiPath(workspace_id): ApiPath<uuid::Uuid>,
) -> ApiResult<Json<Vec<MemberFreeze>>> {
    let workspace_id = WorkspaceId(workspace_id);
    cap(&auth, TOKEN_ADMIN)?;
    ensure_workspace(&auth, workspace_id)?;
    Ok(Json(state.store.list_frozen_members(workspace_id).await?))
}

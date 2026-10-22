//! `/operator/workspaces/{id}/suspension`: an operator suspends a workspace
//! (the hosted service's abuse control) and lifts it. `operator:global`,
//! which provisioned tenants never hold. While suspended, every credential of
//! the workspace is refused (`maidan_auth::ensure_workspace_active`) and its
//! outbound and background work is skipped; its data is kept.

use axum::{extract::State, Extension, Json};
use maidan_auth::capability::OPERATOR_GLOBAL;
use maidan_auth::AuthContext;
use maidan_types::{
    AuditScope, LiftedSuspension, NewAuditEvent, SuspendWorkspace, WorkspaceId, WorkspaceSuspension,
};

use crate::error::ApiError;
use crate::extract::{ApiJson, ApiPath};
use crate::routes::{cap, ApiResult};
use crate::state::AppState;

const REASON_LIMIT: usize = 1000;

/// `GET /operator/workspaces/:id/suspension`: the suspension in force, or 404.
pub async fn get_workspace_suspension(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
) -> ApiResult<Json<WorkspaceSuspension>> {
    cap(&auth, OPERATOR_GLOBAL)?;
    state
        .store
        .get_workspace_suspension(WorkspaceId(id))
        .await?
        .map(Json)
        .ok_or(ApiError::NotFound)
}

/// `PUT /operator/workspaces/:id/suspension`: suspend, or replace the reason
/// of a standing suspension. An operator may not suspend its own workspace:
/// that would lock out the one credential able to lift it.
pub async fn suspend_workspace(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
    ApiJson(body): ApiJson<SuspendWorkspace>,
) -> ApiResult<Json<WorkspaceSuspension>> {
    cap(&auth, OPERATOR_GLOBAL)?;
    let workspace_id = WorkspaceId(id);
    if !auth.bypass && workspace_id == auth.workspace_id {
        return Err(ApiError::BadRequest(
            "an operator may not suspend its own workspace".into(),
        ));
    }
    let reason = body.reason.trim();
    if reason.is_empty() || reason.chars().count() > REASON_LIMIT {
        return Err(ApiError::BadRequest(format!(
            "reason is 1 to {REASON_LIMIT} characters"
        )));
    }
    let actor = (!auth.bypass).then_some(auth.actor_id);
    let suspension = state
        .store
        .suspend_workspace(
            workspace_id,
            reason,
            actor,
            NewAuditEvent {
                scope: AuditScope::Workspace(workspace_id),
                actor_id: actor,
                action: "workspace.suspend".into(),
                target_kind: Some("workspace".into()),
                target_id: Some(workspace_id.0),
                metadata: serde_json::json!({ "reason": reason }),
            },
        )
        .await?;
    Ok(Json(suspension))
}

/// `DELETE /operator/workspaces/:id/suspension`: lift it. `lifted: false` when
/// the workspace was not suspended, and then nothing is audited.
pub async fn lift_workspace_suspension(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
) -> ApiResult<Json<LiftedSuspension>> {
    cap(&auth, OPERATOR_GLOBAL)?;
    let workspace_id = WorkspaceId(id);
    let actor = (!auth.bypass).then_some(auth.actor_id);
    let lifted = state
        .store
        .lift_workspace_suspension(
            workspace_id,
            NewAuditEvent {
                scope: AuditScope::Workspace(workspace_id),
                actor_id: actor,
                action: "workspace.suspension_lift".into(),
                target_kind: Some("workspace".into()),
                target_id: Some(workspace_id.0),
                metadata: serde_json::json!({}),
            },
        )
        .await?;
    Ok(Json(LiftedSuspension { lifted }))
}

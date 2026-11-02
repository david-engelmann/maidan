//! Human invites into an existing workspace: mint, list and revoke. The
//! invitee accepts through the OIDC front door (`/auth/oidc/login?invite=`),
//! in `crate::oidc::invite`; this module never creates a member.

use axum::{extract::State, http::StatusCode, Extension, Json};
use chrono::{Duration, Utc};
use maidan_auth::{
    capability::{WORKSPACE_ADMIN, WORKSPACE_WRITE},
    hash_secret, AuthContext, WorkspaceInviteSecret,
};
use maidan_types::{
    AuditScope, NewAuditEvent, NewWorkspaceInvite, WorkspaceId, WorkspaceInvite, WorkspaceInviteId,
    WorkspaceInviteRole, WORKSPACE_INVITE_DEFAULT_TTL_SECS,
};

use super::{cap, ensure_workspace, ApiResult};
use crate::dto::{CreateWorkspaceInvite, MintWorkspaceInviteResponse};
use crate::error::ApiError;
use crate::extract::{ApiJson, ApiPath};
use crate::state::AppState;

/// The highest role this caller may hand out. A caller without
/// `workspace:write` can only invite viewers, and a member who joined by
/// invite can never invite above the role that invite gave them.
async fn inviter_ceiling(state: &AppState, auth: &AuthContext) -> ApiResult<WorkspaceInviteRole> {
    let by_caps = if auth.has_capability(WORKSPACE_WRITE) {
        WorkspaceInviteRole::Member
    } else {
        WorkspaceInviteRole::Viewer
    };
    let by_membership = state
        .store
        .member_invite_role(auth.member_id)
        .await?
        .unwrap_or(WorkspaceInviteRole::Member);
    Ok(by_caps.min(by_membership))
}

pub async fn create_workspace_invite(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(wid): ApiPath<uuid::Uuid>,
    ApiJson(body): ApiJson<CreateWorkspaceInvite>,
) -> ApiResult<(StatusCode, Json<MintWorkspaceInviteResponse>)> {
    let workspace_id = WorkspaceId(wid);
    cap(&auth, WORKSPACE_ADMIN)?;
    ensure_workspace(&auth, workspace_id)?;
    if body.role > inviter_ceiling(&state, &auth).await? {
        return Err(ApiError::Forbidden(format!(
            "an invite cannot grant the {} role: it is above your own",
            body.role.as_str()
        )));
    }
    let email = body.email.trim().to_lowercase();
    if !maidan_store::workspace_invites::is_plausible_email(&email) {
        return Err(ApiError::BadRequest("email is not a valid address".into()));
    }
    crate::rate_limit::charge_email_domain(
        &state,
        crate::rate_limit::DomainCharge::WorkspaceInviteMint,
        &email,
    )
    .await?;
    let expires_at = body
        .expires_at
        .unwrap_or_else(|| Utc::now() + Duration::seconds(WORKSPACE_INVITE_DEFAULT_TTL_SECS));
    let secret = WorkspaceInviteSecret::generate();
    let actor = auth.actor_id;
    let invite = state
        .store
        .create_workspace_invite_audited(
            NewWorkspaceInvite {
                workspace_id,
                email,
                role: body.role,
                created_by: auth.member_id,
                token_hash: hash_secret(secret.as_str()),
                expires_at,
            },
            Box::new(move |invite: &WorkspaceInvite| NewAuditEvent {
                scope: AuditScope::Workspace(workspace_id),
                actor_id: Some(actor),
                action: "workspace_invite.create".into(),
                target_kind: Some("workspace_invite".into()),
                target_id: Some(invite.id.0),
                // The address stays out of the audit row: the invite row
                // carries it, and erasing the invite erases it.
                metadata: serde_json::json!({
                    "workspace_id": workspace_id.0,
                    "role": invite.role.as_str(),
                    "expires_at": invite.expires_at,
                }),
            }),
        )
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(MintWorkspaceInviteResponse {
            invite,
            accept_path: format!("/auth/oidc/login?invite={}", secret.as_str()),
            secret: secret.as_str().to_owned(),
        }),
    ))
}

pub async fn list_workspace_invites(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(wid): ApiPath<uuid::Uuid>,
) -> ApiResult<Json<Vec<WorkspaceInvite>>> {
    let workspace_id = WorkspaceId(wid);
    cap(&auth, WORKSPACE_ADMIN)?;
    ensure_workspace(&auth, workspace_id)?;
    Ok(Json(
        state.store.list_workspace_invites(workspace_id).await?,
    ))
}

pub async fn revoke_workspace_invite(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath((wid, iid)): ApiPath<(uuid::Uuid, uuid::Uuid)>,
) -> ApiResult<StatusCode> {
    let workspace_id = WorkspaceId(wid);
    let invite_id = WorkspaceInviteId(iid);
    cap(&auth, WORKSPACE_ADMIN)?;
    ensure_workspace(&auth, workspace_id)?;
    if !state
        .store
        .revoke_workspace_invite_audited(
            workspace_id,
            invite_id,
            NewAuditEvent {
                scope: AuditScope::Workspace(workspace_id),
                actor_id: Some(auth.actor_id),
                action: "workspace_invite.revoke".into(),
                target_kind: Some("workspace_invite".into()),
                target_id: Some(invite_id.0),
                metadata: serde_json::json!({ "workspace_id": workspace_id.0 }),
            },
        )
        .await?
    {
        return Err(ApiError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

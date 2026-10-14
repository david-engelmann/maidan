//! Grant management (Next 23, phase four): a member lists the OAuth
//! grants clients hold on them in a workspace, and revokes one.
//!
//! Every read is scoped to the caller's own member id in the workspace
//! named in the path: another member's grant — or the same member's
//! grant in another workspace — is a 404, never a filtered-out row the
//! response admits to knowing about. Revoking ends the grant's whole
//! lineage (refresh and access tokens alike) through the same audited
//! lineage revocation the revocation endpoint uses.

use axum::{extract::State, Extension, Json};
use maidan_auth::{
    capability::{WORKSPACE_READ, WORKSPACE_WRITE},
    AuthContext,
};
use maidan_types::{OAuthGrant, OAuthGrantId, WorkspaceId};
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

use super::{cap, ensure_workspace, ApiResult};
use crate::error::ApiError;
use crate::extract::ApiPath;
use crate::state::AppState;

/// One grant as the member sees it. `client_name` comes from the
/// client's registry row (a metadata-document client gets its row on
/// first use at `/authorize`); it is absent only for a row that predates
/// the name being recorded, and then the grant shows by client id alone.
#[derive(Debug, Serialize, ToSchema)]
pub struct OAuthGrantView {
    pub id: Uuid,
    pub client_id: String,
    pub client_name: Option<String>,
    pub scope: Vec<String>,
    pub lineage_id: Uuid,
}

impl From<&OAuthGrant> for OAuthGrantView {
    fn from(grant: &OAuthGrant) -> Self {
        OAuthGrantView {
            id: grant.id.0,
            client_id: grant.client_id.clone(),
            client_name: None,
            scope: grant.scope.clone(),
            lineage_id: grant.lineage_id,
        }
    }
}

/// What a revoke ended, counted by the lineage revocation itself.
#[derive(Debug, Serialize, ToSchema)]
pub struct RevokeOAuthGrantResponse {
    pub grant_id: Uuid,
    pub lineage_id: Uuid,
    pub access_tokens_revoked: u64,
    pub refresh_tokens_revoked: u64,
}

pub async fn list_oauth_grants(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(workspace_id): ApiPath<Uuid>,
) -> ApiResult<Json<Vec<OAuthGrantView>>> {
    cap(&auth, WORKSPACE_READ)?;
    let workspace_id = WorkspaceId(workspace_id);
    ensure_workspace(&auth, workspace_id)?;
    let grants = state
        .store
        .list_oauth_grants(workspace_id, auth.member_id)
        .await?;
    let mut views = Vec::with_capacity(grants.len());
    for grant in &grants {
        let mut view = OAuthGrantView::from(grant);
        if let Some(client) = state
            .store
            .get_oauth_client_by_client_id(&grant.client_id)
            .await?
        {
            view.client_name = Some(client.name);
        }
        views.push(view);
    }
    Ok(Json(views))
}

pub async fn revoke_oauth_grant(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath((workspace_id, grant_id)): ApiPath<(Uuid, Uuid)>,
) -> ApiResult<Json<RevokeOAuthGrantResponse>> {
    cap(&auth, WORKSPACE_WRITE)?;
    let workspace_id = WorkspaceId(workspace_id);
    ensure_workspace(&auth, workspace_id)?;
    // `get_oauth_grant` returns live grants only. The workspace and
    // member checks are the tenant boundary: a grant that fails either
    // is indistinguishable from one that does not exist.
    let grant = state
        .store
        .get_oauth_grant(OAuthGrantId(grant_id))
        .await?
        .filter(|grant| grant.workspace_id == workspace_id && grant.member_id == auth.member_id)
        .ok_or(ApiError::NotFound)?;
    let revoked = state
        .store
        .revoke_oauth_lineage_audited(
            grant.lineage_id,
            crate::oauth::token::lineage_audit_as(
                "member_revocation",
                grant.client_id.clone(),
                Some(auth.member_id),
            ),
        )
        .await?;
    Ok(Json(RevokeOAuthGrantResponse {
        grant_id: grant.id.0,
        lineage_id: grant.lineage_id,
        access_tokens_revoked: revoked.access_tokens_revoked,
        refresh_tokens_revoked: revoked.refresh_tokens_revoked,
    }))
}

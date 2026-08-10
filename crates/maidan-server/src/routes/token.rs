//! API-token handlers: mint, list, and revoke bearer tokens.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use maidan_auth::{
    capability::{self, TOKEN_ADMIN},
    hash_secret, AuthContext, TokenSecret,
};
use maidan_types::*;

use super::{cap, ensure_workspace, ApiResult};
use crate::dto::*;
use crate::error::{ApiError, ApiJson};
use crate::state::AppState;

pub async fn mint_api_token(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path((workspace_id, member_id)): Path<(uuid::Uuid, uuid::Uuid)>,
    ApiJson(body): ApiJson<MintApiToken>,
) -> ApiResult<(StatusCode, Json<MintApiTokenResponse>)> {
    let workspace_id = WorkspaceId(workspace_id);
    let member_id = MemberId(member_id);
    cap(&auth, TOKEN_ADMIN)?;
    ensure_workspace(&auth, workspace_id)?;

    let member = state.store.get_member(member_id).await?;
    if member.workspace_id != workspace_id {
        return Err(ApiError::BadRequest(
            "member does not belong to workspace".into(),
        ));
    }

    let capabilities = if body.capabilities.is_empty() {
        capability::default_minted()
    } else {
        capability::validate_list(&body.capabilities).map_err(ApiError::BadRequest)?;
        body.capabilities
    };
    crate::quota::validate_token_quotas(&body.quotas, &capabilities)?;

    let secret = TokenSecret::generate();
    let record = state
        .store
        .create_api_token(NewApiToken {
            workspace_id,
            member_id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: body.label,
            capabilities: capabilities.clone(),
            expires_at: body.expires_at,
        })
        .await?;

    if !body.quotas.is_empty() {
        state
            .store
            .replace_token_quotas(record.id, &body.quotas)
            .await?;
    }
    let quotas = state.store.list_token_quotas(record.id).await?;

    crate::audit::record(
        &state,
        NewAuditEvent {
            actor_id: Some(auth.member_id),
            action: "token.mint".into(),
            target_kind: Some("api_token".into()),
            target_id: Some(record.id.0),
            metadata: serde_json::json!({
                "workspace_id": record.workspace_id.0,
                "subject_member_id": record.member_id.0,
                "capabilities": record.capabilities.clone(),
                "expires_at": record.expires_at,
            }),
        },
    )
    .await;

    Ok((
        StatusCode::CREATED,
        Json(MintApiTokenResponse {
            id: record.id,
            secret: secret.as_str().to_string(),
            workspace_id: record.workspace_id,
            member_id: record.member_id,
            capabilities: record.capabilities,
            expires_at: record.expires_at,
            quotas,
        }),
    ))
}

pub async fn list_api_tokens(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path((workspace_id, member_id)): Path<(uuid::Uuid, uuid::Uuid)>,
) -> ApiResult<Json<Vec<crate::dto::ApiTokenSummary>>> {
    let workspace_id = WorkspaceId(workspace_id);
    let member_id = MemberId(member_id);
    cap(&auth, TOKEN_ADMIN)?;
    ensure_workspace(&auth, workspace_id)?;
    let member = state.store.get_member(member_id).await?;
    if member.workspace_id != workspace_id {
        return Err(ApiError::BadRequest(
            "member does not belong to workspace".into(),
        ));
    }
    let tokens = state
        .store
        .list_api_tokens_for_member(workspace_id, member_id)
        .await?;
    Ok(Json(
        tokens
            .into_iter()
            .map(|t| crate::dto::ApiTokenSummary {
                id: t.id,
                workspace_id: t.workspace_id,
                member_id: t.member_id,
                label: t.label,
                capabilities: t.capabilities,
                created_at: t.created_at,
                expires_at: t.expires_at,
                revoked_at: t.revoked_at,
            })
            .collect(),
    ))
}

pub async fn revoke_api_token(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<ApiToken>> {
    cap(&auth, TOKEN_ADMIN)?;
    let token_id = ApiTokenId(id);
    let existing = state.store.get_api_token(token_id).await?;
    ensure_workspace(&auth, existing.workspace_id)?;
    let revoked = state.store.revoke_api_token(token_id).await?;
    crate::audit::record(
        &state,
        NewAuditEvent {
            actor_id: Some(auth.member_id),
            action: "token.revoke".into(),
            target_kind: Some("api_token".into()),
            target_id: Some(revoked.id.0),
            metadata: serde_json::json!({
                "workspace_id": revoked.workspace_id.0,
                "subject_member_id": revoked.member_id.0,
            }),
        },
    )
    .await;
    Ok(Json(revoked))
}

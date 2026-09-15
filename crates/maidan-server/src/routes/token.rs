//! API-token handlers: mint, list, revoke, and holder-side attenuation.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use chrono::{DateTime, Utc};
use maidan_auth::{
    capability::{self, TOKEN_ADMIN, WORKSPACE_READ},
    hash_secret, AuthContext, TokenSecret,
};
use maidan_store::Store;
use maidan_types::*;

use super::{cap, ensure_workspace, ApiResult};
use crate::dto::*;
use crate::error::{ApiError, ApiJson};
use crate::state::AppState;

fn holder_grant(auth: &AuthContext) -> Vec<String> {
    if auth.bypass {
        capability::all()
    } else {
        auth.capabilities().to_vec()
    }
}

async fn parent_expires_at(
    store: &dyn Store,
    auth: &AuthContext,
) -> ApiResult<Option<DateTime<Utc>>> {
    match auth.token_id {
        Some(id) => Ok(store.get_api_token(id).await?.expires_at),
        None => Ok(None),
    }
}

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

    // token:admin may grant any known set or list (held = vocabulary).
    // Holder-side attenuation is POST /tokens/attenuate — no token:admin.
    let capabilities = if body.capability_set.is_none() && body.capabilities.is_empty() {
        capability::default_minted()
    } else {
        maidan_auth::progressive_grant(
            &capability::all(),
            body.capability_set.as_deref(),
            &body.capabilities,
        )
        .map_err(ApiError::BadRequest)?
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
                "capability_set": body.capability_set,
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

/// Named capability-set catalog (`maidan.agent.worker`, `maidan.human.admin`).
pub async fn list_capability_sets(
    Extension(auth): Extension<AuthContext>,
) -> ApiResult<Json<Vec<CapabilitySetView>>> {
    cap(&auth, WORKSPACE_READ)?;
    Ok(Json(
        maidan_auth::named_sets()
            .into_iter()
            .map(|s| CapabilitySetView {
                name: s.name,
                capabilities: s.capabilities,
            })
            .collect(),
    ))
}

/// Levy/Madden holder-side attenuation: derive a weaker token for the
/// caller. No `token:admin`. Amplification is rejected; a derived expiry
/// cannot outlive the parent bearer.
pub async fn attenuate_api_token(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiJson(body): ApiJson<AttenuateToken>,
) -> ApiResult<(StatusCode, Json<MintApiTokenResponse>)> {
    cap(&auth, WORKSPACE_READ)?;
    let capabilities = maidan_auth::attenuate(&holder_grant(&auth), &body.capabilities)
        .map_err(ApiError::BadRequest)?;
    let parent = parent_expires_at(state.store.as_ref(), &auth).await?;
    let expires_at = maidan_auth::attenuate_expiry(parent, body.expires_at, Utc::now())
        .map_err(ApiError::BadRequest)?;

    let secret = TokenSecret::generate();
    let record = state
        .store
        .create_api_token(NewApiToken {
            workspace_id: auth.workspace_id,
            member_id: auth.member_id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: body.label,
            capabilities: capabilities.clone(),
            expires_at,
        })
        .await?;

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
                "attenuated": true,
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
            quotas: Vec::new(),
        }),
    ))
}

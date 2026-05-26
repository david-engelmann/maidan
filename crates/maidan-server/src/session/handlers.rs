use axum::{extract::State, http::StatusCode, Extension, Json};
use chrono::Utc;
use maidan_auth::{capability, hash_secret, TokenSecret, TOKEN_ADMIN};
use maidan_types::NewApiToken;

use crate::dto::{MintApiTokenResponse, SessionResponse};
use crate::error::ApiError;
use crate::session::SessionContext;
use crate::state::AppState;

pub async fn mint_first_admin_token(
    State(state): State<AppState>,
    Extension(ctx): Extension<SessionContext>,
) -> Result<(StatusCode, Json<MintApiTokenResponse>), ApiError> {
    let oidc = state
        .oidc
        .as_ref()
        .ok_or_else(|| ApiError::Forbidden("OIDC is not enabled".into()))?;
    if !oidc.settings.first_admin_mint {
        return Err(ApiError::Forbidden(
            "first-admin session mint is disabled (MAIDAN_OIDC_FIRST_ADMIN)".into(),
        ));
    }
    if state
        .store
        .workspace_has_active_capability(ctx.workspace_id, TOKEN_ADMIN)
        .await?
    {
        return Err(ApiError::Forbidden(
            "workspace already has a token:admin holder".into(),
        ));
    }

    let mut capabilities = capability::default_minted();
    capabilities.push(TOKEN_ADMIN.to_string());
    let secret = TokenSecret::generate();
    let record = state
        .store
        .create_api_token(NewApiToken {
            workspace_id: ctx.workspace_id,
            member_id: ctx.member_id,
            token_hash: hash_secret(secret.as_str()),
            label: Some("oidc-first-admin".into()),
            capabilities,
            expires_at: None,
        })
        .await?;

    Ok((
        StatusCode::CREATED,
        Json(MintApiTokenResponse {
            id: record.id,
            secret: secret.as_str().to_string(),
            workspace_id: record.workspace_id,
            member_id: record.member_id,
            capabilities: record.capabilities,
            expires_at: record.expires_at,
        }),
    ))
}

pub async fn get_session(
    State(state): State<AppState>,
    Extension(ctx): Extension<SessionContext>,
) -> Result<Json<SessionResponse>, ApiError> {
    let session = state.store.get_session(ctx.session_id).await?;
    if session.expires_at < Utc::now() {
        let _ = state.store.delete_session(session.id).await;
        return Err(ApiError::Unauthorized);
    }
    Ok(Json(SessionResponse {
        member_id: session.member_id,
        workspace_id: session.workspace_id,
        expires_at: session.expires_at,
    }))
}

//! OAuth-style authorization code flow for installed apps (Cluster 65).

use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{Duration, Instant};

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use maidan_auth::{capability, hash_secret, AuthContext, TokenSecret};
use maidan_types::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::dto::MintAppTokenResponse;
use crate::error::{ApiError, ApiJson};
use crate::routes::{cap, ensure_workspace};
use crate::state::AppState;

type ApiResult<T> = Result<T, ApiError>;

#[derive(Clone)]
pub struct AppOAuthRuntime {
    codes: std::sync::Arc<RwLock<HashMap<String, PendingAppCode>>>,
}

#[derive(Clone)]
struct PendingAppCode {
    app_id: AppId,
    workspace_id: WorkspaceId,
    redirect_uri: String,
    code_challenge: Option<String>,
    expires_at: Instant,
}

impl Default for AppOAuthRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl AppOAuthRuntime {
    pub fn new() -> Self {
        Self {
            codes: std::sync::Arc::new(RwLock::new(HashMap::new())),
        }
    }

    fn prune(&self) {
        if let Ok(mut guard) = self.codes.write() {
            guard.retain(|_, row| row.expires_at > Instant::now());
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct AuthorizeAppInstall {
    pub redirect_uri: String,
    pub state: String,
    #[serde(default)]
    pub code_challenge: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AuthorizeAppInstallResponse {
    pub authorization_code: String,
    pub state: String,
    pub expires_in_secs: u64,
}

#[derive(Debug, Deserialize)]
pub struct ExchangeAppCode {
    pub code: String,
    pub redirect_uri: String,
    #[serde(default)]
    pub code_verifier: Option<String>,
}

/// Mint a one-time authorization code (requires `token:admin`).
pub async fn authorize_app_install(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path((workspace_id, app_id)): Path<(Uuid, Uuid)>,
    ApiJson(body): ApiJson<AuthorizeAppInstall>,
) -> ApiResult<(StatusCode, Json<AuthorizeAppInstallResponse>)> {
    let workspace_id = WorkspaceId(workspace_id);
    let app_id = AppId(app_id);
    cap(&auth, capability::TOKEN_ADMIN)?;
    ensure_workspace(&auth, workspace_id)?;

    if body.redirect_uri.trim().is_empty() || body.state.trim().is_empty() {
        return Err(ApiError::BadRequest(
            "redirect_uri and state are required".into(),
        ));
    }

    let app = state.store.get_app(app_id).await?;
    if app.workspace_id != workspace_id {
        return Err(ApiError::BadRequest("app is not in this workspace".into()));
    }

    let oauth = state
        .app_oauth
        .as_ref()
        .ok_or_else(|| ApiError::Internal("app oauth runtime not configured".into()))?;
    oauth.prune();

    let code = Uuid::new_v4().to_string();
    let expires_in_secs = 600;
    let row = PendingAppCode {
        app_id,
        workspace_id,
        redirect_uri: body.redirect_uri.clone(),
        code_challenge: body.code_challenge.clone(),
        expires_at: Instant::now() + Duration::from_secs(expires_in_secs),
    };
    if let Ok(mut guard) = oauth.codes.write() {
        guard.insert(code.clone(), row);
    }

    Ok((
        StatusCode::CREATED,
        Json(AuthorizeAppInstallResponse {
            authorization_code: code,
            state: body.state,
            expires_in_secs,
        }),
    ))
}

/// Exchange authorization code for an app-scoped API token (public).
pub async fn exchange_app_code(
    State(state): State<AppState>,
    ApiJson(body): ApiJson<ExchangeAppCode>,
) -> ApiResult<(StatusCode, Json<MintAppTokenResponse>)> {
    let oauth = state
        .app_oauth
        .as_ref()
        .ok_or_else(|| ApiError::Internal("app oauth runtime not configured".into()))?;

    let pending = {
        oauth.prune();
        let mut guard = oauth
            .codes
            .write()
            .map_err(|_| ApiError::Internal("oauth lock poisoned".into()))?;
        guard.remove(&body.code).ok_or(ApiError::Unauthorized)?
    };

    if pending.expires_at <= Instant::now() {
        return Err(ApiError::Unauthorized);
    }
    if pending.redirect_uri != body.redirect_uri {
        return Err(ApiError::BadRequest("redirect_uri mismatch".into()));
    }
    if let Some(challenge) = &pending.code_challenge {
        let verifier = body
            .code_verifier
            .as_deref()
            .ok_or_else(|| ApiError::BadRequest("code_verifier required".into()))?;
        if s256_challenge(verifier) != *challenge {
            return Err(ApiError::Unauthorized);
        }
    }

    let app = state.store.get_app(pending.app_id).await?;
    let installation = match state
        .store
        .list_app_installations(pending.workspace_id)
        .await?
        .into_iter()
        .find(|row| row.app_id == app.id && row.revoked_at.is_none())
    {
        Some(row) => row,
        None => {
            let bot = state
                .store
                .create_member(NewMember {
                    workspace_id: pending.workspace_id,
                    handle: format!("app:{}", app.slug),
                    display_name: Some(app.name.clone()),
                    kind: MemberKind::Agent,
                })
                .await?;
            state
                .store
                .create_app_installation(NewAppInstallation {
                    app_id: app.id,
                    workspace_id: pending.workspace_id,
                    bot_member_id: bot.id,
                    granted_capabilities: capability::default_minted(),
                })
                .await?
        }
    };

    let secret = TokenSecret::generate();
    let record = state
        .store
        .create_api_token(NewApiToken {
            workspace_id: pending.workspace_id,
            member_id: installation.bot_member_id,
            app_installation_id: Some(installation.id),
            token_hash: hash_secret(secret.as_str()),
            label: Some(format!("oauth:{}", app.slug)),
            capabilities: installation.granted_capabilities.clone(),
            expires_at: None,
        })
        .await?;

    Ok((
        StatusCode::CREATED,
        Json(MintAppTokenResponse {
            id: record.id,
            secret: secret.as_str().to_string(),
            workspace_id: pending.workspace_id,
            app_installation_id: installation.id,
            bot_member_id: installation.bot_member_id,
            capabilities: record.capabilities,
            expires_at: record.expires_at,
            quotas: vec![],
        }),
    ))
}

fn s256_challenge(verifier: &str) -> String {
    let hash = Sha256::digest(verifier.as_bytes());
    URL_SAFE_NO_PAD.encode(hash)
}

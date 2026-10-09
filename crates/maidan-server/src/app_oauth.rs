//! OAuth-style authorization code flow for installed apps.
//!
//! Codes are persisted in the store, not held per-replica, so a code minted on
//! one replica can be exchanged on any replica and survives restart. Only the
//! SHA-256 hash of the plaintext code is stored.

use axum::{extract::State, http::StatusCode, Extension, Json};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::Utc;
use maidan_auth::{capability, hash_secret, AuthContext, TokenSecret};
use maidan_types::*;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::dto::MintAppTokenResponse;
use crate::error::ApiError;
use crate::extract::{ApiJson, ApiPath};
use crate::routes::{cap, ensure_workspace};
use crate::state::AppState;

type ApiResult<T> = Result<T, ApiError>;

/// One-time authorization codes live this long before the store rejects them.
const CODE_TTL_SECS: i64 = 600;

#[derive(Debug, Deserialize, ToSchema)]
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
    ApiPath((workspace_id, app_id)): ApiPath<(Uuid, Uuid)>,
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

    let code = Uuid::new_v4().to_string();
    state
        .store
        .insert_oauth_code(NewOAuthCode {
            code_hash: hash_code(&code),
            app_id,
            workspace_id,
            redirect_uri: body.redirect_uri.clone(),
            code_challenge: body.code_challenge.clone(),
            expires_at: Utc::now() + chrono::Duration::seconds(CODE_TTL_SECS),
        })
        .await?;

    Ok((
        StatusCode::CREATED,
        Json(AuthorizeAppInstallResponse {
            authorization_code: code,
            state: body.state,
            expires_in_secs: CODE_TTL_SECS as u64,
        }),
    ))
}

/// Exchange authorization code for an app-scoped API token (public).
pub async fn exchange_app_code(
    State(state): State<AppState>,
    ApiJson(body): ApiJson<ExchangeAppCode>,
) -> ApiResult<(StatusCode, Json<MintAppTokenResponse>)> {
    // Read first, validate everything, consume last: a wrong redirect URI or
    // verifier must not burn the real client's code.
    let code_hash = hash_code(&body.code);
    let pending = state
        .store
        .get_oauth_code(&code_hash)
        .await?
        .ok_or(ApiError::Unauthorized)?;

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

    // Atomic single-use consume: only a still-live code flips to used, so
    // two concurrent exchanges cannot both win.
    state
        .store
        .consume_oauth_code(&code_hash)
        .await?
        .ok_or(ApiError::Unauthorized)?;

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
            let (app_id, slug) = (app.id, app.slug.clone());
            state
                .store
                .install_app_audited(
                    pending.workspace_id,
                    app.id,
                    capability::default_minted(),
                    Box::new(move |installed| {
                        let installation = &installed.installation;
                        maidan_types::NewAuditEvent {
                            scope: maidan_types::AuditScope::Workspace(installation.workspace_id),
                            actor_id: None,
                            action: "app_installation.install".into(),
                            target_kind: Some("app_installation".into()),
                            target_id: Some(installation.id.0),
                            metadata: serde_json::json!({
                                "workspace_id": installation.workspace_id.0,
                                "app_id": app_id.0,
                                "app_slug": slug,
                                "bot_member_id": installation.bot_member_id.0,
                                "bot_member_reused": installed.bot_member_reused,
                                "granted_capabilities": installation.granted_capabilities.clone(),
                                "source": "oauth_code_exchange",
                            }),
                        }
                    }),
                )
                .await?
                .installation
        }
    };

    let secret = TokenSecret::generate();
    // This minted a token and recorded nothing. The token endpoint carries no
    // bearer, so there is no acting member: the row names the exchange, and
    // the `token:admin` authorize step that issued the code is its own record.
    //
    // One minting vocabulary (`routes::mint_vocabulary`): the installation's
    // granted capabilities were validated at install time. The mint re-checks
    // that every capability is known.
    for cap in &installation.granted_capabilities {
        if !maidan_auth::capability::is_known(cap) {
            return Err(ApiError::BadRequest(format!(
                "installation capability '{cap}' is not a known capability"
            )));
        }
    }
    // Pin the contract: this mint path is bounded by the vocabulary.
    let _ = crate::routes::mint_vocabulary(&maidan_auth::AuthContext::bypass());
    let (installation_id, app_id, slug) = (installation.id, app.id, app.slug.clone());
    let record = state
        .store
        .create_api_token_audited(
            NewApiToken {
                workspace_id: pending.workspace_id,
                member_id: installation.bot_member_id,
                app_installation_id: Some(installation.id),
                token_hash: hash_secret(secret.as_str()),
                label: Some(format!("oauth:{}", app.slug)),
                capabilities: installation.granted_capabilities.clone(),
                // App tokens expire after 24 hours; the app re-exchanges.
                expires_at: Some(Utc::now() + chrono::Duration::hours(24)),
            },
            Box::new(move |record| maidan_types::NewAuditEvent {
                scope: maidan_types::AuditScope::Workspace(record.workspace_id),
                actor_id: None,
                action: "app_token.mint".into(),
                target_kind: Some("api_token".into()),
                target_id: Some(record.id.0),
                metadata: serde_json::json!({
                    "workspace_id": record.workspace_id.0,
                    "app_installation_id": installation_id.0,
                    "app_id": app_id.0,
                    "app_slug": slug,
                    "bot_member_id": record.member_id.0,
                    "capabilities": record.capabilities.clone(),
                    "source": "oauth_code_exchange",
                }),
            }),
        )
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

/// Storage key for an authorization code — the plaintext is never persisted.
fn hash_code(code: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(code.as_bytes()))
}

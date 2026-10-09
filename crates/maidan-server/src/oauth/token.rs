//! `POST /oauth/token` — the OAuth 2.1 token endpoint (`docs/OAuth.md`,
//! phase two).
//!
//! Exchanges an authorization code for a capability-scoped access token. The
//! code is read and fully validated (client, redirect URI, resource, PKCE
//! verifier) before it is consumed, so a wrong guess cannot burn the real
//! client's code; the consume itself is one atomic statement. The minted
//! token carries the grant's scopes and lineage, and never `approval:grant`
//! (stripped at mint, so an OAuth token can never accept an approval gate).

use axum::{extract::State, http::StatusCode, response::IntoResponse, Json};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::Utc;
use maidan_auth::token::hashes_equal;
use maidan_auth::{hash_secret, TokenSecret};
use maidan_types::{AuditScope, NewApiToken, NewAuditEvent, NewOAuthGrant};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::error::ApiError;
use crate::extract::ApiForm;
use crate::state::AppState;

type ApiResult<T> = Result<T, ApiError>;

/// Access tokens minted here live this long.
const ACCESS_TOKEN_TTL_SECS: i64 = 3600;

#[derive(Debug, Deserialize, ToSchema)]
pub struct OAuthTokenRequest {
    pub grant_type: String,
    pub code: String,
    pub redirect_uri: String,
    pub client_id: String,
    pub code_verifier: String,
    pub client_secret: Option<String>,
    /// RFC 8707 resource indicator; must match the code's when set.
    pub resource: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct OAuthTokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: i64,
    pub scope: String,
}

/// Exchange an authorization code for an access token (public; the client
/// authenticates with its secret and the PKCE verifier).
///
/// Error contract: this endpoint uses the repository's RFC 9457 problem-details
/// contract (`ApiError`), not OAuth-style `{"error": "invalid_grant"}` bodies.
/// A bad code, expired code, or wrong verifier is a 401; a mismatched
/// client_id, redirect_uri, or resource is a 400. Maidan's MCP clients speak
/// this contract; the endpoint is not advertised as a generic OAuth token
/// endpoint for third-party clients.
pub async fn token(
    State(state): State<AppState>,
    ApiForm(form): ApiForm<OAuthTokenRequest>,
) -> ApiResult<impl IntoResponse> {
    if form.grant_type != "authorization_code" {
        return Err(ApiError::BadRequest(
            "grant_type must be authorization_code".into(),
        ));
    }

    let code_hash = URL_SAFE_NO_PAD.encode(Sha256::digest(form.code.as_bytes()));
    // Read first, validate everything, consume last: a wrong verifier must
    // not burn the real client's code.
    let pending = state
        .store
        .get_oauth_authorization_code(&code_hash)
        .await?
        .ok_or(ApiError::Unauthorized)?;
    if pending.expires_at <= Utc::now() || pending.used_at.is_some() {
        return Err(ApiError::Unauthorized);
    }
    if pending.client_id != form.client_id {
        return Err(ApiError::BadRequest("client_id mismatch".into()));
    }
    if pending.redirect_uri != form.redirect_uri {
        return Err(ApiError::BadRequest("redirect_uri mismatch".into()));
    }
    if pending.resource != form.resource {
        return Err(ApiError::BadRequest("resource mismatch".into()));
    }

    let client = state
        .store
        .get_oauth_client_by_client_id(&form.client_id)
        .await?
        .ok_or(ApiError::Unauthorized)?;
    if let Some(expected) = client.client_secret_hash.as_deref() {
        let provided = form
            .client_secret
            .as_deref()
            .ok_or(ApiError::Unauthorized)?;
        if !hashes_equal(expected, &hash_secret(provided)) {
            return Err(ApiError::Unauthorized);
        }
    }

    // PKCE, S256 only: the challenge stored at authorize time must be the
    // SHA-256 of this verifier.
    if URL_SAFE_NO_PAD.encode(Sha256::digest(form.code_verifier.as_bytes()))
        != pending.code_challenge
    {
        return Err(ApiError::Unauthorized);
    }

    // Atomic single-use consume: only a still-live code flips to used, so
    // two concurrent exchanges cannot both win.
    state
        .store
        .consume_oauth_authorization_code(&code_hash)
        .await?
        .ok_or(ApiError::Unauthorized)?;

    // The authorize step records the grant; a direct exchange still works by
    // creating it here. Either way the token is minted under the grant, so
    // revoking the grant later stops the token.
    let grant = match state
        .store
        .find_oauth_grant(
            &pending.client_id,
            pending.member_id,
            pending.workspace_id,
            &pending.scope,
        )
        .await?
    {
        Some(grant) => grant,
        None => {
            let (client_id, scope_meta, workspace_id) = (
                pending.client_id.clone(),
                pending.scope.clone(),
                pending.workspace_id,
            );
            state
                .store
                .create_oauth_grant_audited(
                    NewOAuthGrant {
                        client_id: pending.client_id.clone(),
                        member_id: pending.member_id,
                        workspace_id: pending.workspace_id,
                        scope: pending.scope.clone(),
                        lineage_id: Uuid::now_v7(),
                    },
                    Box::new(move |grant| NewAuditEvent {
                        scope: AuditScope::Workspace(workspace_id),
                        actor_id: None,
                        action: "oauth_grant.create".into(),
                        target_kind: Some("oauth_grant".into()),
                        target_id: Some(grant.id.0),
                        metadata: serde_json::json!({
                            "client_id": client_id,
                            "scope": scope_meta,
                            "lineage_id": grant.lineage_id,
                            "source": "oauth_token_exchange",
                        }),
                    }),
                )
                .await?
        }
    };

    // The token endpoint carries no bearer, so there is no acting member:
    // the row names the exchange, like the installed-app flow.
    //
    // One minting vocabulary (`routes::mint_vocabulary`): the grant's scopes
    // were validated as delegatable at authorize time. The mint re-checks
    // that every scope is a known capability — the full vocabulary, which is
    // what `mint_vocabulary` returns for a bypass context.
    for cap in &grant.scope {
        if !maidan_auth::capability::is_known(cap) {
            return Err(ApiError::BadRequest(format!(
                "grant scope '{cap}' is not a known capability"
            )));
        }
    }
    // Pin the contract: this mint path is bounded by the vocabulary.
    let _ = crate::routes::mint_vocabulary(&maidan_auth::AuthContext::bypass());
    let secret = TokenSecret::generate();
    let (grant_id, client_id, lineage_id, scope_meta) = (
        grant.id,
        grant.client_id.clone(),
        grant.lineage_id,
        grant.scope.clone(),
    );
    let record = state
        .store
        .mint_oauth_token_audited(
            NewApiToken {
                workspace_id: grant.workspace_id,
                member_id: grant.member_id,
                app_installation_id: None,
                token_hash: hash_secret(secret.as_str()),
                label: Some(format!("oauth:{}", grant.client_id)),
                capabilities: grant.scope.clone(),
                expires_at: Some(Utc::now() + chrono::Duration::seconds(ACCESS_TOKEN_TTL_SECS)),
            },
            grant.id,
            Box::new(move |record| NewAuditEvent {
                scope: AuditScope::Workspace(record.workspace_id),
                actor_id: None,
                action: "oauth_token.mint".into(),
                target_kind: Some("api_token".into()),
                target_id: Some(record.id.0),
                metadata: serde_json::json!({
                    "client_id": client_id,
                    "oauth_grant_id": grant_id.0,
                    "lineage_id": lineage_id,
                    "capabilities": scope_meta,
                    "source": "oauth_code_exchange",
                }),
            }),
        )
        .await?;

    Ok((
        StatusCode::OK,
        [
            (
                axum::http::header::CACHE_CONTROL,
                axum::http::HeaderValue::from_static("no-store"),
            ),
            (
                axum::http::header::PRAGMA,
                axum::http::HeaderValue::from_static("no-cache"),
            ),
        ],
        Json(OAuthTokenResponse {
            access_token: secret.as_str().to_string(),
            token_type: "Bearer".to_string(),
            expires_in: ACCESS_TOKEN_TTL_SECS,
            scope: record.capabilities.join(" "),
        }),
    ))
}

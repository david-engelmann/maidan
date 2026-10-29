//! `POST /oauth/token`: the OAuth 2.1 token endpoint (`docs/OAuth.md`, phases
//! two and three).
//!
//! Two grants. `authorization_code` exchanges a code for an access token and
//! a refresh token. The code is read and fully validated (client, redirect
//! URI, resource, PKCE verifier) before it is consumed, so a wrong guess
//! cannot burn the real client's code. A code presented again after it was
//! used revokes the lineage of the grant it minted under.
//!
//! `refresh_token` rotates: the presented refresh token is marked used and a
//! new pair is minted under the same grant and lineage, in one transaction.
//! A refresh token presented after it was used is a reuse, which revokes the
//! whole lineage (OAuth 2.1 section 4.3.1).
//!
//! Every token is bound to the MCP resource (RFC 8707): the request must name
//! it, and the bearer middleware accepts the access token nowhere else. No
//! token carries `approval:grant` (stripped at mint, so an OAuth token can
//! never accept an approval gate). Errors are RFC 6749 section 5.2 errors,
//! not problems: this is the endpoint the RFC 8414 document advertises to
//! OAuth clients.

use axum::{
    extract::{Request, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::Utc;
use maidan_auth::{hash_secret, TokenSecret};
use maidan_types::{
    ApiToken, AuditScope, NewApiToken, NewAuditEvent, NewOAuthRefreshToken, OAuthClient,
    OAuthGrant, OAuthLineageRevocation, OAuthRefreshOutcome, OAuthRefreshRotation,
};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use utoipa::ToSchema;

use super::client_auth::{self, ClientCredentials};
use super::error::{OAuthError, OAuthErrorCode, OAuthForm};
use crate::error::ApiError;
use crate::state::AppState;

/// Access tokens minted here live this long.
const ACCESS_TOKEN_TTL_SECS: i64 = 3600;

/// Refresh tokens minted here live this long. Each rotation issues a new one
/// with a fresh lifetime, so a client in use keeps its lineage alive.
pub const REFRESH_TOKEN_TTL_DAYS: i64 = 30;

/// Every parameter either grant reads. All optional, so a missing one is an
/// `invalid_request` naming it rather than a deserialization failure.
#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct OAuthTokenRequest {
    /// `authorization_code` or `refresh_token`.
    pub grant_type: Option<String>,
    /// The authorization code (`authorization_code`).
    pub code: Option<String>,
    /// The redirect URI the code was issued for (`authorization_code`).
    pub redirect_uri: Option<String>,
    /// Required unless the client authenticates with HTTP Basic.
    pub client_id: Option<String>,
    /// The PKCE verifier (`authorization_code`).
    pub code_verifier: Option<String>,
    /// `client_secret_post`; never beside an `Authorization` header.
    pub client_secret: Option<String>,
    /// RFC 8707 resource indicator: must be the MCP endpoint,
    /// `<MAIDAN_PUBLIC_ORIGIN>/mcp/streamable`.
    pub resource: Option<String>,
    /// The refresh token (`refresh_token`).
    pub refresh_token: Option<String>,
    /// Space-delimited; on `refresh_token` it may narrow the grant's scope,
    /// never widen it.
    pub scope: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct OAuthTokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: i64,
    pub refresh_token: String,
    pub scope: String,
}

/// Off until `MAIDAN_PUBLIC_ORIGIN` is set, like the discovery documents: the
/// resource tokens are bound to is named from the origin, so without it there
/// is nothing to issue a token for. A layer, so the 404 comes before the body
/// is read.
pub async fn require_public_origin(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Response {
    if state.public_origin.is_none() {
        return ApiError::NotFound.into_response();
    }
    next.run(req).await
}

/// The token endpoint. Public: the client authenticates with HTTP Basic, its
/// secret in the body, or, for a public client, PKCE alone.
pub async fn token(
    State(state): State<AppState>,
    headers: HeaderMap,
    OAuthForm(form): OAuthForm<OAuthTokenRequest>,
) -> Result<Response, OAuthError> {
    let origin = state
        .public_origin
        .clone()
        .ok_or_else(|| OAuthError::new(OAuthErrorCode::ServerError, "no public origin"))?;
    let grant_type = form
        .grant_type
        .as_deref()
        .filter(|g| !g.is_empty())
        .ok_or_else(|| OAuthError::invalid_request("grant_type is required"))?;
    if !matches!(grant_type, "authorization_code" | "refresh_token") {
        return Err(OAuthError::new(
            OAuthErrorCode::UnsupportedGrantType,
            "grant_type must be authorization_code or refresh_token",
        ));
    }
    let creds = client_auth::credentials(
        &headers,
        form.client_id.as_deref(),
        form.client_secret.as_deref(),
    )?;
    // A pre-registered client is authenticated before the code or token is
    // read, so a wrong secret learns nothing about them.
    let preregistered = client_auth::authenticate_preregistered(&state, &creds).await?;
    if preregistered.is_none() {
        client_auth::require_known_shape(&creds)?;
    }
    if grant_type == "authorization_code" {
        exchange_code(&state, &origin, &creds, preregistered, form).await
    } else {
        refresh(&state, &origin, &creds, preregistered, form).await
    }
}

async fn exchange_code(
    state: &AppState,
    origin: &str,
    creds: &ClientCredentials,
    preregistered: Option<OAuthClient>,
    form: OAuthTokenRequest,
) -> Result<Response, OAuthError> {
    let code = required(form.code, "code")?;
    let redirect_uri = required(form.redirect_uri, "redirect_uri")?;
    let verifier = required(form.code_verifier, "code_verifier")?;
    let code_hash = opaque_hash(&code);

    // Read first, validate everything, consume last: a wrong verifier must
    // not burn the real client's code.
    let pending = state
        .store
        .get_oauth_authorization_code(&code_hash)
        .await?
        .ok_or_else(|| OAuthError::invalid_grant("unknown authorization code"))?;
    if pending.client_id != creds.client_id {
        return Err(OAuthError::invalid_grant(
            "the code was issued to another client",
        ));
    }
    if pending.used_at.is_some() {
        // Only a presenter holding the verifier revokes: the code alone is
        // useless under PKCE, and letting it revoke would let anyone who saw
        // a redirect end the member's session.
        if opaque_hash(&verifier) == pending.code_challenge {
            revoke_for_reused_code(state, &code_hash, &creds.client_id).await?;
        }
        return Err(OAuthError::invalid_grant("the code was already used"));
    }
    let client = client_auth::authenticate_in(state, creds, preregistered).await?;
    if pending.expires_at <= Utc::now() {
        return Err(OAuthError::invalid_grant("the code has expired"));
    }
    if pending.redirect_uri != redirect_uri {
        return Err(OAuthError::invalid_grant(
            "redirect_uri is not the one the code was issued for",
        ));
    }
    let resource = require_resource(form.resource.as_deref(), origin)?;
    if pending.resource.as_deref() != Some(resource.as_str()) {
        return Err(OAuthError::invalid_grant(
            "the code was not issued for this resource",
        ));
    }
    // PKCE, S256 only: the challenge stored at authorize time must be the
    // SHA-256 of this verifier.
    if opaque_hash(&verifier) != pending.code_challenge {
        return Err(OAuthError::invalid_grant(
            "code_verifier does not match the code challenge",
        ));
    }
    client_allows(&client, &pending.scope)?;

    // Atomic single-use consume: only a still-live code flips to used, so two
    // concurrent exchanges cannot both win. The loser presented a used code.
    if state
        .store
        .consume_oauth_authorization_code(&code_hash)
        .await?
        .is_none()
    {
        revoke_for_reused_code(state, &code_hash, &creds.client_id).await?;
        return Err(OAuthError::invalid_grant("the code is no longer valid"));
    }

    let grant = grant_for(state, &pending).await?;
    for cap in &grant.scope {
        if !maidan_auth::capability::is_known(cap) {
            return Err(OAuthError::new(
                OAuthErrorCode::InvalidScope,
                format!("grant scope {cap:?} is not a known capability"),
            ));
        }
    }
    // One minting vocabulary (`routes::mint_vocabulary`): the grant's scopes
    // were validated as delegatable at authorize time and are re-checked as
    // known capabilities above, which is what it returns for a bypass context.
    let _ = crate::routes::mint_vocabulary(&maidan_auth::AuthContext::bypass());

    let access = TokenSecret::generate();
    let (refresh_secret, refresh_hash) = new_refresh_token();
    let record = state
        .store
        .mint_oauth_token_pair_audited(
            NewApiToken {
                workspace_id: grant.workspace_id,
                member_id: grant.member_id,
                app_installation_id: None,
                token_hash: hash_secret(access.as_str()),
                label: Some(format!("oauth:{}", grant.client_id)),
                capabilities: grant.scope.clone(),
                expires_at: Some(Utc::now() + chrono::Duration::seconds(ACCESS_TOKEN_TTL_SECS)),
            },
            grant.id,
            NewOAuthRefreshToken {
                token_hash: refresh_hash,
                grant_id: grant.id,
                lineage_id: grant.lineage_id,
                client_id: grant.client_id.clone(),
                resource,
                expires_at: Utc::now() + chrono::Duration::days(REFRESH_TOKEN_TTL_DAYS),
            },
            mint_audit(&grant, "oauth_code_exchange"),
        )
        .await?;
    Ok(token_response(access.as_str(), &refresh_secret, &record))
}

/// The grant a code mints under: the live one consent recorded. None is a
/// revoked grant, whether by the member or by a reuse of this very code that
/// raced the exchange, and the code dies with it rather than starting a new
/// lineage the revocation never saw.
async fn grant_for(
    state: &AppState,
    pending: &maidan_types::OAuthAuthorizationCode,
) -> Result<OAuthGrant, OAuthError> {
    state
        .store
        .find_oauth_grant(
            &pending.client_id,
            pending.member_id,
            pending.workspace_id,
            &pending.scope,
        )
        .await?
        .ok_or_else(|| OAuthError::invalid_grant("the grant behind this code was revoked"))
}

async fn refresh(
    state: &AppState,
    origin: &str,
    creds: &ClientCredentials,
    preregistered: Option<OAuthClient>,
    form: OAuthTokenRequest,
) -> Result<Response, OAuthError> {
    let presented = required(form.refresh_token, "refresh_token")?;
    let presented_hash = opaque_hash(&presented);
    let stored = state
        .store
        .get_oauth_refresh_token(&presented_hash)
        .await?
        .ok_or_else(|| OAuthError::invalid_grant("unknown refresh token"))?;
    if stored.client_id != creds.client_id {
        return Err(OAuthError::invalid_grant(
            "the refresh token was issued to another client",
        ));
    }
    if stored.used_at.is_some() {
        state
            .store
            .revoke_oauth_lineage_audited(
                stored.lineage_id,
                lineage_audit("refresh_token_reuse", creds.client_id.clone()),
            )
            .await?;
        return Err(OAuthError::invalid_grant(
            "the refresh token was already used, so its lineage is revoked",
        ));
    }
    if stored.revoked_at.is_some() || stored.expires_at <= Utc::now() {
        return Err(OAuthError::invalid_grant(
            "the refresh token is expired or revoked",
        ));
    }
    let grant = state
        .store
        .get_oauth_grant(stored.grant_id)
        .await?
        .ok_or_else(|| {
            OAuthError::invalid_grant("the grant behind the refresh token is revoked")
        })?;
    let client = client_auth::authenticate_in(state, creds, preregistered).await?;
    let resource = require_resource(form.resource.as_deref(), origin)?;
    if resource != stored.resource {
        return Err(OAuthError::invalid_target(
            "resource is not the one the refresh token is bound to",
        ));
    }
    let scope = narrowed_scope(form.scope.as_deref(), &grant.scope)?;
    client_allows(&client, &scope)?;

    let access = TokenSecret::generate();
    let (refresh_secret, refresh_hash) = new_refresh_token();
    let outcome = state
        .store
        .rotate_oauth_refresh_token(
            OAuthRefreshRotation {
                presented_hash,
                access_token_hash: hash_secret(access.as_str()),
                access_label: Some(format!("oauth:{}", grant.client_id)),
                access_capabilities: scope,
                access_expires_at: Utc::now() + chrono::Duration::seconds(ACCESS_TOKEN_TTL_SECS),
                refresh_hash,
                refresh_expires_at: Utc::now() + chrono::Duration::days(REFRESH_TOKEN_TTL_DAYS),
            },
            mint_audit(&grant, "oauth_refresh"),
            lineage_audit("refresh_token_reuse", creds.client_id.clone()),
        )
        .await?;
    match outcome {
        OAuthRefreshOutcome::Rotated { access: record, .. } => {
            Ok(token_response(access.as_str(), &refresh_secret, &record))
        }
        OAuthRefreshOutcome::Reused(_) => Err(OAuthError::invalid_grant(
            "the refresh token was already used, so its lineage is revoked",
        )),
        OAuthRefreshOutcome::Invalid => Err(OAuthError::invalid_grant(
            "the refresh token is expired or revoked",
        )),
    }
}

/// A used code was presented again: revoke the lineage of the grant it
/// minted under. Nothing to revoke is not an error, the request is refused
/// either way.
async fn revoke_for_reused_code(
    state: &AppState,
    code_hash: &str,
    client_id: &str,
) -> Result<(), OAuthError> {
    state
        .store
        .revoke_oauth_lineage_for_used_code(
            code_hash,
            lineage_audit("code_reuse", client_id.into()),
        )
        .await?;
    Ok(())
}

/// The audit row of a lineage revocation.
pub(super) fn lineage_audit(
    reason: &'static str,
    client_id: String,
) -> maidan_store::AuditFor<OAuthLineageRevocation> {
    lineage_audit_as(reason, client_id, None)
}

/// [`lineage_audit`] naming the member who ended the lineage, for the
/// member-facing grant management surface: a client revoking its own
/// token has no member actor; a member revoking a grant does.
pub(crate) fn lineage_audit_as(
    reason: &'static str,
    client_id: String,
    actor_id: Option<maidan_types::MemberId>,
) -> maidan_store::AuditFor<OAuthLineageRevocation> {
    Box::new(move |revoked| NewAuditEvent {
        scope: revoked
            .workspace_id
            .map_or(AuditScope::Instance, AuditScope::Workspace),
        actor_id,
        action: "oauth_lineage.revoke".into(),
        target_kind: Some("oauth_lineage".into()),
        target_id: Some(revoked.lineage_id),
        metadata: serde_json::json!({
            "reason": reason,
            "client_id": client_id,
            "grants_revoked": revoked.grants_revoked,
            "access_tokens_revoked": revoked.access_tokens_revoked,
            "refresh_tokens_revoked": revoked.refresh_tokens_revoked,
        }),
    })
}

/// The token endpoint carries no bearer, so there is no acting member: the
/// row names the exchange, like the installed-app flow.
fn mint_audit(grant: &OAuthGrant, source: &'static str) -> maidan_store::AuditFor<ApiToken> {
    let (grant_id, client_id, lineage_id) = (grant.id, grant.client_id.clone(), grant.lineage_id);
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
            "capabilities": record.capabilities,
            "source": source,
        }),
    })
}

fn token_response(access: &str, refresh: &str, record: &ApiToken) -> Response {
    (
        StatusCode::OK,
        [
            (header::CACHE_CONTROL, HeaderValue::from_static("no-store")),
            (header::PRAGMA, HeaderValue::from_static("no-cache")),
        ],
        Json(OAuthTokenResponse {
            access_token: access.to_owned(),
            token_type: "Bearer".to_owned(),
            expires_in: ACCESS_TOKEN_TTL_SECS,
            refresh_token: refresh.to_owned(),
            scope: record.capabilities.join(" "),
        }),
    )
        .into_response()
}

fn required(value: Option<String>, name: &str) -> Result<String, OAuthError> {
    value
        .filter(|v| !v.is_empty())
        .ok_or_else(|| OAuthError::invalid_request(format!("{name} is required")))
}

/// RFC 8707: the request must name the one resource tokens are issued for.
pub(super) fn require_resource(given: Option<&str>, origin: &str) -> Result<String, OAuthError> {
    let expected = super::metadata::mcp_resource(origin);
    match given.filter(|r| !r.is_empty()) {
        Some(resource) if resource == expected => Ok(expected),
        Some(_) => Err(OAuthError::invalid_target(format!(
            "this server issues tokens only for the resource {expected}"
        ))),
        None => Err(OAuthError::invalid_target(format!(
            "resource is required and must be {expected}"
        ))),
    }
}

/// The scope a refreshed token carries: the grant's, or the narrower set the
/// request names. A scope the grant does not hold is `invalid_scope`
/// (RFC 6749 section 6), so a refresh can never widen a grant.
fn narrowed_scope(requested: Option<&str>, granted: &[String]) -> Result<Vec<String>, OAuthError> {
    let Some(requested) = requested else {
        return Ok(granted.to_vec());
    };
    let mut scope: Vec<String> = requested
        .split(' ')
        .filter(|c| !c.is_empty())
        .map(str::to_owned)
        .collect();
    if scope.is_empty() {
        return Err(OAuthError::new(
            OAuthErrorCode::InvalidScope,
            "scope must name at least one capability",
        ));
    }
    if let Some(wider) = scope.iter().find(|c| !granted.contains(c)) {
        return Err(OAuthError::new(
            OAuthErrorCode::InvalidScope,
            format!("scope {wider:?} was not granted, and a refresh cannot widen a grant"),
        ));
    }
    scope.sort();
    scope.dedup();
    Ok(scope)
}

/// The client's registration must still allow every scope it is about to
/// receive: an operator who narrows a client stops tokens it was already
/// granted from being minted or refreshed past the new limit.
fn client_allows(client: &OAuthClient, scope: &[String]) -> Result<(), OAuthError> {
    match scope.iter().find(|c| !client.allowed_scopes.contains(c)) {
        Some(cap) => Err(OAuthError::new(
            OAuthErrorCode::UnauthorizedClient,
            format!("the client is no longer allowed the scope {cap:?}"),
        )),
        None => Ok(()),
    }
}

/// The stored form of a code, verifier or refresh token: unpadded base64url
/// SHA-256, so the secret itself is never stored.
pub(super) fn opaque_hash(secret: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(secret.as_bytes()))
}

/// A new refresh token and its stored hash.
fn new_refresh_token() -> (String, String) {
    let mut raw = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut raw);
    let token = URL_SAFE_NO_PAD.encode(raw);
    let hash = opaque_hash(&token);
    (token, hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refresh_scope_narrows_and_never_widens() {
        let granted = vec!["workspace:read".to_string(), "workspace:write".to_string()];
        assert_eq!(narrowed_scope(None, &granted).unwrap(), granted);
        assert_eq!(
            narrowed_scope(Some("workspace:read workspace:read"), &granted).unwrap(),
            vec!["workspace:read".to_string()]
        );
        for wider in ["message:post", "workspace:read token:mint", " "] {
            let err = narrowed_scope(Some(wider), &granted).unwrap_err();
            assert_eq!(err.code, OAuthErrorCode::InvalidScope, "{wider:?}");
        }
    }

    #[test]
    fn the_resource_must_be_the_mcp_endpoint_of_the_configured_origin() {
        let origin = "https://maidan.example.com";
        assert_eq!(
            require_resource(Some("https://maidan.example.com/mcp/streamable"), origin).unwrap(),
            "https://maidan.example.com/mcp/streamable"
        );
        for bad in [
            None,
            Some(""),
            Some("https://maidan.example.com/mcp"),
            Some("https://other.example.com/mcp/streamable"),
        ] {
            let err = require_resource(bad, origin).unwrap_err();
            assert_eq!(err.code, OAuthErrorCode::InvalidTarget, "{bad:?}");
        }
    }
}

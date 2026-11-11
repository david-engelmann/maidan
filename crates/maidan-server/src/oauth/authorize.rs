//! `GET /oauth/authorize` — the OAuth 2.1 authorization endpoint
//! (`docs/OAuth.md`, phase two).
//!
//! The client redirects the member's browser here. The bearer middleware ran
//! before this route: the member is authenticated by a bearer or a signed-in
//! session, and a browser with no credential at all was redirected to sign in
//! and brought back ([`sign_in_redirect`]). This step
//! validates everything — client, redirect URI (exact, or a loopback one on
//! any port), S256 PKCE, scopes as
//! delegatable capabilities the member holds and the client allows — then
//! stores a pending request and redirects to the consent page. No code is
//! issued here; the member approves or denies with a same-origin POST on
//! their signed-in session (`oauth::consent`).

use axum::{
    extract::State,
    http::{header, StatusCode},
    response::{IntoResponse, Redirect, Response},
    Extension,
};
use chrono::Utc;
use maidan_auth::{capability, AuthContext};
use maidan_types::NewOAuthPendingRequest;
use serde::Deserialize;

use crate::error::ApiError;
use crate::extract::ApiQuery;
use crate::state::AppState;

type ApiResult<T> = Result<T, ApiError>;

/// Pending consent requests live this long before the consent page refuses
/// them.
const PENDING_TTL_SECS: i64 = 600;

#[derive(Debug, Deserialize)]
pub struct AuthorizeQuery {
    pub client_id: String,
    pub redirect_uri: String,
    pub code_challenge: String,
    pub code_challenge_method: String,
    /// Space-delimited capabilities. Defaults to everything the client is
    /// allowed when omitted.
    pub scope: Option<String>,
    pub state: String,
    /// RFC 8707 resource indicator the grant is bound to. When given it must
    /// be `<MAIDAN_PUBLIC_ORIGIN>/mcp/streamable`; when omitted, that is the one.
    pub resource: Option<String>,
    /// OAuth 2.1 requires `response_type=code`.
    pub response_type: String,
}

/// Where a signed-out browser at `/oauth/authorize` goes: the sign-in, with
/// `return_to` set to the full authorize path and query so the browser comes
/// back to this request once signed in. `None` when the instance has no
/// sign-in to send it to, which leaves the 401.
pub fn sign_in_redirect(state: &AppState, uri: &axum::http::Uri) -> Option<Response> {
    state.oidc.as_ref()?;
    let return_to = uri
        .path_and_query()
        .map(|pq| pq.as_str())
        .unwrap_or(super::metadata::AUTHORIZE_PATH);
    let location = format!(
        "/auth/oidc/login?return_to={}",
        urlencoding::encode(return_to)
    );
    Some(
        (
            StatusCode::FOUND,
            [
                (header::LOCATION, location),
                (header::CACHE_CONTROL, "no-store".to_string()),
            ],
        )
            .into_response(),
    )
}

/// Validate the authorization request, store it pending the member's
/// consent decision, and redirect to the consent page. Never issues a code.
pub async fn authorize(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiQuery(query): ApiQuery<AuthorizeQuery>,
) -> ApiResult<impl IntoResponse> {
    // The authorization server is off until the deployment names its origin,
    // which the resource every grant is bound to is built from.
    let origin = state.public_origin.as_deref().ok_or(ApiError::NotFound)?;
    // Authorizing a client lends the member's authority; a credential that
    // cannot read the workspace has none worth lending.
    crate::routes::cap(&auth, capability::WORKSPACE_READ)?;
    // OAuth 2.1 requires response_type=code.
    if query.response_type != "code" {
        return Err(ApiError::BadRequest("response_type must be code".into()));
    }

    // P1: pre-registered clients first, then CIMD via the fetch guard.
    let client = super::registry::resolve_client(&state, &query.client_id).await?;

    // Exact match, no prefix games or path confusion, except that a loopback
    // redirect's port may vary (RFC 8252 section 7.3): a native app listens on
    // whatever port its OS hands it.
    if !super::registry::redirect_uri_matches(&client.redirect_uris, &query.redirect_uri) {
        return Err(ApiError::BadRequest("redirect_uri mismatch".into()));
    }

    // PKCE is S256-only. `plain` and anything else are refused outright.
    if query.code_challenge_method != "S256" {
        return Err(ApiError::BadRequest(
            "code_challenge_method must be S256".into(),
        ));
    }
    // Anything but the 43 characters an S256 digest encodes to is not a
    // challenge some verifier can meet, and storing it would issue a code
    // that no exchange could redeem.
    if !super::token::is_s256_challenge(&query.code_challenge) {
        return Err(ApiError::BadRequest(
            "code_challenge must be an S256 challenge: the 43-character unpadded base64url \
             SHA-256 of the code_verifier"
                .into(),
        ));
    }
    if query.state.trim().is_empty() {
        return Err(ApiError::BadRequest("state is required".into()));
    }

    let requested: Vec<String> = query
        .scope
        .as_deref()
        .unwrap_or("")
        .split(' ')
        .filter(|c| !c.is_empty())
        .map(str::to_string)
        .collect();
    // A request that names no scope (claude.ai's connector sends none) asks
    // for what the client may have, cut to what the member can lend. A scope
    // named outright is still refused below when the member lacks it.
    let mut scope: Vec<String> = if requested.is_empty() {
        client
            .allowed_scopes
            .iter()
            .filter(|c| capability::is_delegatable(c) && auth.has_capability(c))
            .cloned()
            .collect()
    } else {
        requested
    };
    if scope.is_empty() {
        return Err(ApiError::BadRequest("scope is empty".into()));
    }
    for cap in &scope {
        // A scope names a delegatable capability only: never `token:*`,
        // `operator:*`, `approval:grant` or `audit:read-global`.
        if !capability::is_delegatable(cap) {
            return Err(ApiError::BadRequest(format!(
                "scope {cap:?} is not a delegatable capability"
            )));
        }
        if !client.allowed_scopes.contains(cap) {
            return Err(ApiError::BadRequest(format!(
                "scope {cap:?} is not allowed for this client"
            )));
        }
        if !auth.has_capability(cap) {
            return Err(ApiError::Forbidden(format!(
                "scope {cap:?} exceeds the member's capabilities"
            )));
        }
    }
    scope.sort();
    scope.dedup();

    // RFC 8707: every grant is for the MCP resource, so the code and every
    // token minted from it carry the one resource they are bound to. A
    // request that names no resource gets that one (`require_resource`):
    // claude.ai's connector names none, and refusing it refused every
    // claude.ai sign-in.
    let resource = super::token::require_resource(query.resource.as_deref(), origin)
        .map_err(|err| ApiError::BadRequest(format!("invalid_target: {}", err.description)))?;

    // Store the validated request pending the member's decision. The grant
    // and the code are created only when the member approves on the consent
    // page (`oauth::consent`), never here.
    let pending = state
        .store
        .create_oauth_pending_request(NewOAuthPendingRequest {
            client_id: client.client_id.clone(),
            member_id: auth.member_id,
            workspace_id: auth.workspace_id,
            redirect_uri: query.redirect_uri,
            code_challenge: query.code_challenge,
            scope,
            resource: Some(resource),
            state: query.state,
            expires_at: Utc::now() + chrono::Duration::seconds(PENDING_TTL_SECS),
        })
        .await?;

    let location = format!("/ui/oauth/consent?request={}", pending.id.0);
    Ok(Redirect::temporary(&location).into_response())
}

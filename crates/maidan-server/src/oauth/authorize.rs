//! `GET /oauth/authorize` — the OAuth 2.1 authorization endpoint
//! (`docs/OAuth.md`, phase two).
//!
//! The client redirects the member's browser here. The member is already
//! authenticated (the bearer middleware ran before this route). This step
//! validates everything — client, exact redirect URI, S256 PKCE, scopes as
//! delegatable capabilities the member holds and the client allows — then
//! stores a pending request and redirects to the consent page. No code is
//! issued here; the member approves or denies with a same-origin POST on
//! their signed-in session (`oauth::consent`).

use axum::{
    extract::State,
    response::{IntoResponse, Redirect},
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
    /// RFC 8707 resource indicator the grant is bound to.
    pub resource: Option<String>,
}

/// Validate the authorization request, store it pending the member's
/// consent decision, and redirect to the consent page. Never issues a code.
pub async fn authorize(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiQuery(query): ApiQuery<AuthorizeQuery>,
) -> ApiResult<impl IntoResponse> {
    // P1: pre-registered clients first, then CIMD via the egress guard.
    let client =
        super::registry::resolve_client(&state, auth.workspace_id, &query.client_id).await?;

    // Exact match: no prefix games, no path confusion.
    if !client.redirect_uris.contains(&query.redirect_uri) {
        return Err(ApiError::BadRequest("redirect_uri mismatch".into()));
    }

    // PKCE is S256-only. `plain` and anything else are refused outright.
    if query.code_challenge_method != "S256" {
        return Err(ApiError::BadRequest(
            "code_challenge_method must be S256".into(),
        ));
    }
    if query.code_challenge.trim().is_empty() {
        return Err(ApiError::BadRequest("code_challenge is required".into()));
    }
    if query.state.trim().is_empty() {
        return Err(ApiError::BadRequest("state is required".into()));
    }

    let mut scope: Vec<String> = match query.scope {
        Some(s) => s
            .split(' ')
            .filter(|c| !c.is_empty())
            .map(str::to_string)
            .collect(),
        None => client.allowed_scopes.clone(),
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

    if let Some(resource) = query.resource.as_deref() {
        if resource.trim().is_empty() {
            return Err(ApiError::BadRequest("resource must not be blank".into()));
        }
    }

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
            resource: query.resource,
            state: query.state,
            expires_at: Utc::now() + chrono::Duration::seconds(PENDING_TTL_SECS),
        })
        .await?;

    let location = format!("/ui/oauth/consent?request={}", pending.id.0);
    Ok(Redirect::temporary(&location).into_response())
}

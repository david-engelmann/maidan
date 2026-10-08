//! `GET /oauth/authorize` — the OAuth 2.1 authorization endpoint
//! (`docs/OAuth.md`, phase two).
//!
//! The client redirects the member's browser here. The member is already
//! authenticated (the bearer middleware ran before this route), so this is
//! the consent step: the requested scopes must be delegatable capabilities
//! the member actually holds and the client is allowed. Phase two
//! auto-approves — the consent page in `/ui` arrives with P4 — but the grant
//! is recorded either way.
//!
//! The issued code is single-use, bound to the client, redirect URI, PKCE
//! challenge and resource, and expires in ten minutes.

use axum::{
    extract::State,
    response::{IntoResponse, Redirect},
    Extension,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::Utc;
use maidan_auth::{capability, AuthContext};
use maidan_types::{AuditScope, NewAuditEvent, NewOAuthAuthorizationCode, NewOAuthGrant};
use rand::RngCore;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::error::ApiError;
use crate::extract::ApiQuery;
use crate::state::AppState;

type ApiResult<T> = Result<T, ApiError>;

/// Codes live this long before the token endpoint refuses them.
const CODE_TTL_SECS: i64 = 600;

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

/// Issue an authorization code for a validated request, recording the
/// member's grant, then redirect back to the client.
pub async fn authorize(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiQuery(query): ApiQuery<AuthorizeQuery>,
) -> ApiResult<impl IntoResponse> {
    let client = state
        .store
        .get_oauth_client_by_client_id(&query.client_id)
        .await?
        .ok_or_else(|| ApiError::BadRequest("unknown client_id".into()))?;

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

    // One grant per client, member, workspace and scope set: a second
    // authorize with the same parameters reuses it instead of piling up rows.
    if state
        .store
        .find_oauth_grant(&client.client_id, auth.member_id, auth.workspace_id, &scope)
        .await?
        .is_none()
    {
        let (client_id, scope_meta, workspace_id, member_id) = (
            client.client_id.clone(),
            scope.clone(),
            auth.workspace_id,
            auth.member_id,
        );
        state
            .store
            .create_oauth_grant_audited(
                NewOAuthGrant {
                    client_id: client.client_id.clone(),
                    member_id: auth.member_id,
                    workspace_id: auth.workspace_id,
                    scope: scope.clone(),
                    lineage_id: Uuid::now_v7(),
                },
                Box::new(move |grant| NewAuditEvent {
                    scope: AuditScope::Workspace(workspace_id),
                    actor_id: Some(member_id),
                    action: "oauth_grant.create".into(),
                    target_kind: Some("oauth_grant".into()),
                    target_id: Some(grant.id.0),
                    metadata: serde_json::json!({
                        "client_id": client_id,
                        "scope": scope_meta,
                        "lineage_id": grant.lineage_id,
                        "member_id": member_id.0,
                    }),
                }),
            )
            .await?;
    }

    let mut raw = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut raw);
    let code = URL_SAFE_NO_PAD.encode(raw);
    state
        .store
        .create_oauth_authorization_code(NewOAuthAuthorizationCode {
            code_hash: URL_SAFE_NO_PAD.encode(Sha256::digest(code.as_bytes())),
            client_id: client.client_id,
            member_id: auth.member_id,
            workspace_id: auth.workspace_id,
            redirect_uri: query.redirect_uri.clone(),
            code_challenge: query.code_challenge,
            scope,
            resource: query.resource,
            expires_at: Utc::now() + chrono::Duration::seconds(CODE_TTL_SECS),
        })
        .await?;

    let separator = if query.redirect_uri.contains('?') {
        '&'
    } else {
        '?'
    };
    let location = format!(
        "{}{}code={}&state={}",
        query.redirect_uri,
        separator,
        urlencoding::encode(&code),
        urlencoding::encode(&query.state),
    );
    Ok(Redirect::temporary(&location).into_response())
}

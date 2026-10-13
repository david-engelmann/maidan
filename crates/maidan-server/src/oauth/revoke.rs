//! `POST /oauth/revoke`: token revocation (RFC 7009, `docs/OAuth.md` phase
//! three).
//!
//! The client authenticates as at the token endpoint. A refresh token revokes
//! its whole lineage: every grant, access token and refresh token under it.
//! An access token revokes that token only. The answer is 200 with an empty
//! body whether or not the token was known, and a token issued to another
//! client is left alone with the same 200, so the endpoint never tells a
//! client anything about a token that is not its own.

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use maidan_auth::hash_secret;
use maidan_types::{AuditScope, NewAuditEvent};
use serde::Deserialize;
use utoipa::ToSchema;

use super::client_auth;
use super::error::{OAuthError, OAuthForm};
use super::token::{lineage_audit, opaque_hash};
use crate::state::AppState;

#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct OAuthRevokeRequest {
    /// The access token or refresh token to revoke.
    pub token: Option<String>,
    /// `access_token` or `refresh_token`: where to look first. Both kinds are
    /// looked up whatever it says (RFC 7009 section 2.1).
    pub token_type_hint: Option<String>,
    /// Required unless the client authenticates with HTTP Basic.
    pub client_id: Option<String>,
    /// `client_secret_post`; never beside an `Authorization` header.
    pub client_secret: Option<String>,
}

#[derive(Clone, Copy)]
enum Kind {
    Refresh,
    Access,
}

/// Revoke a token the authenticating client holds. 200 for a known token, an
/// unknown one and another client's alike.
pub async fn revoke(
    State(state): State<AppState>,
    headers: HeaderMap,
    OAuthForm(form): OAuthForm<OAuthRevokeRequest>,
) -> Result<Response, OAuthError> {
    let creds = client_auth::credentials(
        &headers,
        form.client_id.as_deref(),
        form.client_secret.as_deref(),
    )?;
    let preregistered = client_auth::authenticate_preregistered(&state, &creds).await?;
    // A client metadata document is not fetched here: revoking a token the
    // client already holds can only take authority away.
    if preregistered.is_none() {
        client_auth::require_known_shape(&creds)?;
    }
    let token = form
        .token
        .filter(|t| !t.is_empty())
        .ok_or_else(|| OAuthError::invalid_request("token is required"))?;
    let order = if form.token_type_hint.as_deref() == Some("access_token") {
        [Kind::Access, Kind::Refresh]
    } else {
        [Kind::Refresh, Kind::Access]
    };
    for kind in order {
        let found = match kind {
            Kind::Refresh => revoke_refresh(&state, &creds.client_id, &token).await?,
            Kind::Access => revoke_access(&state, &creds.client_id, &token).await?,
        };
        if found {
            break;
        }
    }
    Ok(StatusCode::OK.into_response())
}

/// `true` when `token` is a refresh token, revoked or not. Only the client it
/// was issued to revokes its lineage.
async fn revoke_refresh(
    state: &AppState,
    client_id: &str,
    token: &str,
) -> Result<bool, OAuthError> {
    let Some(stored) = state
        .store
        .get_oauth_refresh_token(&opaque_hash(token))
        .await?
    else {
        return Ok(false);
    };
    if stored.client_id == client_id {
        state
            .store
            .revoke_oauth_lineage_audited(
                stored.lineage_id,
                lineage_audit("client_revocation", client_id.to_owned()),
            )
            .await?;
    }
    Ok(true)
}

/// `true` when this client's live access token was revoked. Another client's
/// token, or a token no grant issued, is not touched and reads as unknown.
async fn revoke_access(state: &AppState, client_id: &str, token: &str) -> Result<bool, OAuthError> {
    let owner = client_id.to_owned();
    let revoked = state
        .store
        .revoke_oauth_access_token_audited(
            &hash_secret(token),
            client_id,
            Box::new(move |record| NewAuditEvent {
                scope: AuditScope::Workspace(record.workspace_id),
                actor_id: None,
                action: "oauth_token.revoke".into(),
                target_kind: Some("api_token".into()),
                target_id: Some(record.id.0),
                metadata: serde_json::json!({
                    "client_id": owner,
                    "oauth_grant_id": record.oauth_grant_id.map(|g| g.0),
                    "source": "oauth_revocation_endpoint",
                }),
            }),
        )
        .await?;
    Ok(revoked.is_some())
}

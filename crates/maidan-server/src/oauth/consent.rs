//! The OAuth 2.1 consent page (`docs/OAuth.md`, phase two).
//!
//! `GET /oauth/authorize` validates the request and stores a pending
//! request, then redirects here. This page shows the client and the scopes it
//! wants; the member approves or denies with a same-origin POST bound to
//! their signed-in session. A code is never issued on a GET.
//!
//! The session must be one the person signed in to (`SessionContext::token`
//! is `None`): a token-made session or a bearer proves no more than the
//! token, and consent is a human decision.

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse},
    Extension,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::Utc;
use maidan_types::{
    AuditScope, NewAuditEvent, NewOAuthAuthorizationCode, NewOAuthGrant, OAuthPendingRequestId,
};
use rand::RngCore;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::app::BOARD_UI_CSP;
use crate::error::ApiError;
use crate::extract::{ApiForm, ApiQuery};
use crate::oauth::registry::resolve_client;
use crate::session::{require_same_origin, SessionContext};
use crate::state::AppState;

type ApiResult<T> = Result<T, ApiError>;

/// Codes issued after consent live this long before the token endpoint
/// refuses them.
const CODE_TTL_SECS: i64 = 600;

#[derive(Debug, Deserialize)]
pub struct ConsentQuery {
    pub request: String,
}

/// Render the consent page. The pending request must exist, be unexpired,
/// and belong to the session's member. Never issues a code.
pub async fn consent_page(
    State(state): State<AppState>,
    Extension(session): Extension<SessionContext>,
    ApiQuery(query): ApiQuery<ConsentQuery>,
) -> ApiResult<impl IntoResponse> {
    // Consent is a human decision: a token-made session proves no more than
    // the token behind it.
    if session.token.is_some() {
        return Err(ApiError::Forbidden(
            "consent needs a browser session the person signed in to".into(),
        ));
    }
    let request_id = query
        .request
        .parse::<Uuid>()
        .map(OAuthPendingRequestId)
        .map_err(|_| ApiError::BadRequest("bad request id".into()))?;
    let pending = state
        .store
        .get_oauth_pending_request(request_id)
        .await?
        .ok_or(ApiError::NotFound)?;
    if pending.expires_at <= Utc::now() {
        return Err(ApiError::BadRequest("consent request expired".into()));
    }
    if pending.member_id != session.member_id {
        return Err(ApiError::Forbidden(
            "this consent request belongs to another member".into(),
        ));
    }
    let client = resolve_client(&state, pending.workspace_id, &pending.client_id)
        .await
        .map_err(|_| ApiError::BadRequest("unknown client".into()))?;

    let scopes: String = pending
        .scope
        .iter()
        .map(|s| format!("<li><code>{}</code></li>", html_escape(s)))
        .collect::<Vec<_>>()
        .join("");
    // The consent form POSTs to /ui/api/oauth/consent, and the 303 after it
    // navigates to the client's redirect URI. `form-action 'self'` alone
    // would block that navigation, so allow the redirect target's origin.
    let redirect_origin = redirect_origin_for_csp(&pending.redirect_uri);
    let csp = BOARD_UI_CSP.replacen(
        "form-action 'self'",
        &format!("form-action 'self' {redirect_origin}"),
        1,
    );
    let body = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head><meta charset="utf-8"><title>Authorize {client_name}</title></head>
<body>
<h1>Authorize {client_name}?</h1>
<p><strong>{client_name}</strong> wants access to your Maidan workspace with these capabilities:</p>
<ul>{scopes}</ul>
<form method="post" action="/ui/api/oauth/consent">
<input type="hidden" name="request_id" value="{request_id}">
<button type="submit" name="approved" value="true">Allow</button>
<button type="submit" name="approved" value="false">Deny</button>
</form>
</body>
</html>"#,
        client_name = html_escape(&client.name),
        request_id = pending.id.0,
    );
    Ok((
        [(axum::http::header::CONTENT_SECURITY_POLICY, csp)],
        Html(body),
    ))
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[derive(Debug, Deserialize)]
pub struct ConsentDecision {
    pub request_id: String,
    pub approved: bool,
}

/// Decide a pending consent request. Same-origin POST on the signed-in
/// session only. Consumes the pending request (single use). Responds with a
/// 303 redirect to the client's redirect URI (native form POST; no JS).
pub async fn decide_consent(
    State(state): State<AppState>,
    Extension(session): Extension<SessionContext>,
    headers: HeaderMap,
    ApiForm(form): ApiForm<ConsentDecision>,
) -> ApiResult<impl IntoResponse> {
    if session.token.is_some() {
        return Err(ApiError::Forbidden(
            "consent needs a browser session the person signed in to".into(),
        ));
    }
    require_same_origin(&headers)?;

    let request_id = form
        .request_id
        .parse::<Uuid>()
        .map(OAuthPendingRequestId)
        .map_err(|_| ApiError::BadRequest("bad request id".into()))?;
    // Atomic consume: the row is deleted only if it is unexpired and belongs
    // to this member and workspace. Two concurrent POSTs yield exactly one
    // row; anything else (unknown, expired, another member's) is one 404.
    let pending = state
        .store
        .consume_oauth_pending_request(request_id, session.member_id, session.workspace_id)
        .await?
        .ok_or(ApiError::NotFound)?;

    let separator = if pending.redirect_uri.contains('?') {
        '&'
    } else {
        '?'
    };
    if !form.approved {
        let redirect_to = format!(
            "{}{}error=access_denied&state={}",
            pending.redirect_uri,
            separator,
            urlencoding::encode(&pending.state),
        );
        return Ok((
            StatusCode::SEE_OTHER,
            [(axum::http::header::LOCATION, redirect_to)],
        ));
    }

    // Approved: record the grant (reusing an identical one) and issue the
    // single-use code, exactly as the old auto-approve path did. The unique
    // index on live grants makes this atomic: a concurrent approval for the
    // same tuple gets a Conflict, which we treat as "already exists".
    let scope = pending.scope.clone();
    if state
        .store
        .find_oauth_grant(
            &pending.client_id,
            pending.member_id,
            pending.workspace_id,
            &scope,
        )
        .await?
        .is_none()
    {
        let (client_id, scope_meta, workspace_id, member_id) = (
            pending.client_id.clone(),
            scope.clone(),
            pending.workspace_id,
            pending.member_id,
        );
        let result = state
            .store
            .create_oauth_grant_audited(
                NewOAuthGrant {
                    client_id: pending.client_id.clone(),
                    member_id: pending.member_id,
                    workspace_id: pending.workspace_id,
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
            .await;
        // A concurrent consent for the same tuple won the race; the grant
        // exists, so proceed. Any other error is real.
        if let Err(e) = result {
            if !matches!(e, maidan_store::StoreError::Conflict(_)) {
                return Err(e.into());
            }
        }
    }

    let mut raw = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut raw);
    let code = URL_SAFE_NO_PAD.encode(raw);
    state
        .store
        .create_oauth_authorization_code(NewOAuthAuthorizationCode {
            code_hash: URL_SAFE_NO_PAD.encode(Sha256::digest(code.as_bytes())),
            client_id: pending.client_id.clone(),
            member_id: pending.member_id,
            workspace_id: pending.workspace_id,
            redirect_uri: pending.redirect_uri.clone(),
            code_challenge: pending.code_challenge.clone(),
            scope,
            resource: pending.resource.clone(),
            expires_at: Utc::now() + chrono::Duration::seconds(CODE_TTL_SECS),
        })
        .await?;

    let redirect_to = format!(
        "{}{}code={}&state={}",
        pending.redirect_uri,
        separator,
        urlencoding::encode(&code),
        urlencoding::encode(&pending.state),
    );
    Ok((
        StatusCode::SEE_OTHER,
        [(axum::http::header::LOCATION, redirect_to)],
    ))
}

/// The CSP origin for a redirect URI: `scheme://host:port` for a tuple
/// origin, `scheme:` for anything else. `Origin::ascii_serialization()`
/// returns `"null"` for opaque origins (e.g. custom schemes), which would
/// block the navigation; the scheme alone is the correct CSP source.
fn redirect_origin_for_csp(redirect_uri: &str) -> String {
    url::Url::parse(redirect_uri)
        .ok()
        .map(|u| {
            if u.origin().is_tuple() {
                u.origin().ascii_serialization()
            } else {
                format!("{}:", u.scheme())
            }
        })
        .unwrap_or_else(|| "'self'".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn https_redirect_gives_origin() {
        assert_eq!(
            redirect_origin_for_csp("https://client.example/callback?x=1"),
            "https://client.example"
        );
    }

    #[test]
    fn custom_scheme_redirect_gives_scheme() {
        // ascii_serialization() would return "null" here; the CSP needs
        // the scheme.
        assert_eq!(
            redirect_origin_for_csp("myapp://callback"),
            "myapp:"
        );
    }
}

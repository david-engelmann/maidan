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
    AuditScope, Member, MemberKind, NewAuditEvent, NewOAuthAuthorizationCode, NewOAuthGrant,
    OAuthPendingRequestId,
};
use rand::RngCore;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::error::ApiError;
use crate::extract::{ApiForm, ApiQuery};
use crate::oauth::page::{self, html_escape};
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
    let member = require_consenting_person(&state, &session).await?;
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
    let client = resolve_client(&state, &pending.client_id)
        .await
        .map_err(|_| ApiError::BadRequest("unknown client".into()))?;

    let workspace = state.store.get_workspace(session.workspace_id).await?;
    let you = member.display_name.as_deref().unwrap_or(&member.handle);
    // A client that registered itself chose its own name, so the name is
    // marked unverified wherever it shows, and the page says where the code
    // goes and when to refuse.
    let self_registered = client.registered_at.is_some();
    let shown_name = if self_registered {
        format!("{} (unverified app, registered by itself)", client.name)
    } else {
        client.name.clone()
    };
    let client_name = html_escape(&shown_name);
    let returns_to = if self_registered {
        redirect_target(&pending.redirect_uri)
    } else {
        redirect_host(&pending.redirect_uri)
    };
    let warning = if self_registered {
        format!(
            r#"<p class="warn"><b>Maidan has not verified this app.</b> It named itself “{name}” when it registered, and any app can claim any name. Allow only if you just started this sign-in yourself, from an app you trust. If you did not, choose Deny.</p>"#,
            name = html_escape(&client.name),
        )
    } else {
        String::new()
    };
    // The consent form POSTs to /ui/api/oauth/consent, and the 303 after it
    // navigates to the client's redirect URI. `form-action 'self'` alone
    // would block that navigation, so allow the redirect target's origin.
    let redirect_origin = redirect_origin_for_csp(&pending.redirect_uri);
    let csp = page::csp().replacen(
        "form-action 'self'",
        &format!("form-action 'self' {redirect_origin}"),
        1,
    );
    let body = format!(
        r#"<h1>Connect {client_name} to Maidan?</h1>
<p class="sub"><b>{client_name}</b> will act as <b>{you}</b> in the <b>{workspace}</b> workspace.</p>
{warning}
<h2>It will be able to</h2>
{capabilities}
<p class="note">It cannot invite people, change roles, mint tokens or read secrets. You can disconnect it at any time from <a href="/ui/oauth/grants">Connected apps</a>.</p>
<div class="actions"><form method="post" action="/ui/api/oauth/consent">
<input type="hidden" name="request_id" value="{request_id}">
<button type="submit" name="approved" value="false">Deny</button>
<button type="submit" name="approved" value="true" class="primary">Allow</button>
</form></div>
<p class="foot">Either way, you go back to {returns_to}.</p>"#,
        you = html_escape(you),
        workspace = html_escape(&workspace.name),
        capabilities = page::capability_list(&pending.scope),
        returns_to = html_escape(&returns_to),
        request_id = pending.id.0,
    );
    Ok((
        [(axum::http::header::CONTENT_SECURITY_POLICY, csp)],
        Html(page::render(&format!("Connect {shown_name}"), "", &body)),
    ))
}

/// The member who may consent on this session. Consent lends a person's
/// authority, so it takes a person: a session they signed in to (a session
/// made from a token proves no more than the token behind it), whose member
/// is a human still in the workspace. An agent member never consents,
/// however its session came to exist, and neither does a removed member.
async fn require_consenting_person(
    state: &AppState,
    session: &SessionContext,
) -> ApiResult<Member> {
    if session.token.is_some() {
        return Err(ApiError::Forbidden(
            "consent needs a browser session the person signed in to".into(),
        ));
    }
    let member = state.store.get_member(session.member_id).await?;
    if member.kind != MemberKind::Human
        || member.tombstoned_at.is_some()
        || member.workspace_id != session.workspace_id
    {
        return Err(ApiError::Forbidden(
            "consent needs a person: an agent or removed member cannot connect an app".into(),
        ));
    }
    Ok(member)
}

/// Where the redirect goes, as a person reads it: the host for a web
/// address, the scheme for an app's own.
fn redirect_host(redirect_uri: &str) -> String {
    match url::Url::parse(redirect_uri) {
        Ok(u) => u
            .host_str()
            .map(str::to_string)
            .unwrap_or_else(|| format!("{}:", u.scheme())),
        Err(_) => "the app".to_string(),
    }
}

/// Where a self-registered client's redirect goes, in words: the person's
/// own computer for a loopback address, the app that opens a private-use
/// scheme, or the host of an `https` address.
fn redirect_target(redirect_uri: &str) -> String {
    let Ok(url) = url::Url::parse(redirect_uri) else {
        return "the app".to_string();
    };
    match url.scheme() {
        "http" if super::registry::is_loopback_host(&url) => format!(
            "an app on this computer ({}:{})",
            url.host_str().unwrap_or("localhost"),
            url.port_or_known_default().unwrap_or(80)
        ),
        "http" | "https" => url
            .host_str()
            .map(str::to_string)
            .unwrap_or_else(|| "the app".to_string()),
        scheme => format!("the app that opens {scheme}: links"),
    }
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
    require_consenting_person(&state, &session).await?;
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
            "{}{}error=access_denied&state={}{}",
            pending.redirect_uri,
            separator,
            urlencoding::encode(&pending.state),
            super::metadata::issuer_param(state.public_origin.as_deref()),
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
        if let Err(e) = result
            && !matches!(e, maidan_store::StoreError::Conflict(_))
        {
            return Err(e.into());
        }
    }

    // The code records the lineage it is issued under, so a reuse of it later
    // revokes this lineage and not a grant the member consents to afterwards.
    let lineage_id = state
        .store
        .find_oauth_grant(
            &pending.client_id,
            pending.member_id,
            pending.workspace_id,
            &scope,
        )
        .await?
        .map(|grant| grant.lineage_id);

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
            lineage_id,
        })
        .await?;

    let redirect_to = format!(
        "{}{}code={}&state={}{}",
        pending.redirect_uri,
        separator,
        urlencoding::encode(&code),
        urlencoding::encode(&pending.state),
        super::metadata::issuer_param(state.public_origin.as_deref()),
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
    fn a_self_registered_clients_redirect_is_named_for_a_person() {
        assert_eq!(
            redirect_target("http://127.0.0.1:53123/callback"),
            "an app on this computer (127.0.0.1:53123)"
        );
        assert_eq!(
            redirect_target("http://localhost/callback"),
            "an app on this computer (localhost:80)"
        );
        assert_eq!(
            redirect_target("cursor://anysphere.cursor-mcp/oauth/callback"),
            "the app that opens cursor: links"
        );
        assert_eq!(redirect_target("https://vscode.dev/redirect"), "vscode.dev");
    }

    #[test]
    fn custom_scheme_redirect_gives_scheme() {
        // ascii_serialization() would return "null" here; the CSP needs
        // the scheme.
        assert_eq!(redirect_origin_for_csp("myapp://callback"), "myapp:");
    }
}

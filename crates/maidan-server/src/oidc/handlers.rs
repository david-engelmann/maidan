use axum::{
    extract::State,
    http::HeaderMap,
    response::{IntoResponse, Redirect, Response},
};
use chrono::{Duration, Utc};
use maidan_auth::TOKEN_ADMIN;
use maidan_types::{
    AuditScope, NewAuditEvent, NewMaidanSession, NewOidcPendingAuth, OidcPendingTarget, WorkspaceId,
};
use openidconnect::{
    core::CoreAuthenticationFlow, AuthorizationCode, IssuerUrl, LogoutRequest, Nonce,
    PkceCodeChallenge, PkceCodeVerifier, PostLogoutRedirectUrl, Scope, TokenResponse,
};
use rand::RngCore;

use crate::dto::{OidcCallbackQuery, OidcLoginQuery};
use crate::error::ApiError;
use crate::extract::ApiQuery;
use crate::oidc::member::{resolve_member_for_login, touch_identity, NOT_PROVISIONED};
use crate::session::{clear_session_cookie, parse_session_cookie, set_session_cookie};
use crate::state::AppState;

/// The audit action of signing in.
pub const SESSION_CREATE: &str = "session.create";
/// The audit action of signing out.
pub const SESSION_DELETE: &str = "session.delete";

fn random_token() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    base64::Engine::encode(&base64::engine::general_purpose::URL_SAFE_NO_PAD, bytes)
}

fn safe_return_to(return_to: Option<&str>) -> String {
    match return_to {
        Some(path) if path.starts_with('/') && !path.starts_with("//") => path.to_string(),
        _ => "/ui/".to_string(),
    }
}

fn with_hint(location: String, hint: &str) -> String {
    let sep = if location.contains('?') { '&' } else { '?' };
    format!("{location}{sep}{hint}")
}

fn with_auto_mint_hint(location: String) -> String {
    with_hint(location, "auto_mint=1")
}

/// The console's hint, after a front-door sign-in, that the identity has more
/// than one workspace, so it offers the chooser.
pub const CHOOSE_WORKSPACE_HINT: &str = "choose_workspace=1";
/// The console's hint that a front-door sign-in found no workspace for the
/// identity. No session was created.
pub const NO_WORKSPACE_HINT: &str = "no_workspace=1";

pub async fn login(
    State(state): State<AppState>,
    ApiQuery(q): ApiQuery<OidcLoginQuery>,
) -> Result<Response, ApiError> {
    let oidc = state
        .oidc
        .as_ref()
        .ok_or_else(|| ApiError::Forbidden("OIDC is not enabled".into()))?;
    // No lookup here: a workspace id that doesn't exist redirects exactly
    // like one that does, and the callback refuses it with the words a
    // workspace the person isn't a member of gets (Hosted Console, open
    // question 5). Without an id this is the front door.
    let target = match q.workspace_id {
        Some(id) => OidcPendingTarget::Workspace(WorkspaceId(id)),
        None => OidcPendingTarget::FrontDoor,
    };

    let state_token = random_token();
    let nonce = random_token();
    let pkce_verifier = PkceCodeVerifier::new(random_token());
    let pkce_secret = pkce_verifier.secret().to_string();
    let expires_at = Utc::now() + Duration::seconds(oidc.settings.pending_ttl_secs as i64);

    state
        .store
        .insert_oidc_pending(NewOidcPendingAuth {
            state: state_token.clone(),
            target,
            nonce: nonce.clone(),
            pkce_verifier: pkce_secret.clone(),
            return_to: q.return_to.clone(),
            expires_at,
        })
        .await?;

    if oidc.settings.mock {
        let mut url = format!(
            "/auth/oidc/callback?state={}",
            urlencoding::encode(&state_token)
        );
        url.push_str("&mock_sub=mock-user&mock_email=human@example.com");
        return Ok(Redirect::temporary(&url).into_response());
    }

    let client = oidc
        .client
        .as_ref()
        .ok_or_else(|| ApiError::Internal("OIDC client is not configured".into()))?;

    let pkce_challenge = PkceCodeChallenge::from_code_verifier_sha256(&pkce_verifier);
    let scopes: Vec<Scope> = std::env::var("MAIDAN_OIDC_SCOPES")
        .unwrap_or_else(|_| "openid profile email".to_string())
        .split_whitespace()
        .map(|s| Scope::new(s.to_string()))
        .collect();

    let mut req = client.authorize_url(
        CoreAuthenticationFlow::AuthorizationCode,
        || openidconnect::CsrfToken::new(state_token),
        || Nonce::new(nonce),
    );
    req = req.set_pkce_challenge(pkce_challenge);
    for scope in scopes {
        req = req.add_scope(scope);
    }
    let (auth_url, _csrf, _nonce) = req.url();
    Ok(Redirect::temporary(auth_url.as_str()).into_response())
}

pub async fn callback(
    State(state): State<AppState>,
    headers_in: HeaderMap,
    ApiQuery(q): ApiQuery<OidcCallbackQuery>,
) -> Result<Response, ApiError> {
    let oidc = state
        .oidc
        .as_ref()
        .ok_or_else(|| ApiError::Forbidden("OIDC is not enabled".into()))?;

    let pending = state.store.take_oidc_pending(&q.state).await?;

    let (issuer, subject, email, email_verified) = if oidc.settings.mock {
        let sub = q.mock_sub.as_deref().unwrap_or("mock-user").to_string();
        let email = q.mock_email.clone();
        (oidc.settings.issuer.clone(), sub, email, true)
    } else {
        let code = q
            .code
            .as_deref()
            .ok_or_else(|| ApiError::BadRequest("missing authorization code".into()))?;
        let client = oidc
            .client
            .as_ref()
            .ok_or_else(|| ApiError::Internal("OIDC client is not configured".into()))?;
        let http_client = oidc
            .http_client
            .as_ref()
            .ok_or_else(|| ApiError::Internal("OIDC HTTP client is not configured".into()))?;

        let token_response = client
            .exchange_code(AuthorizationCode::new(code.to_string()))
            .map_err(|e| ApiError::BadRequest(e.to_string()))?
            .set_pkce_verifier(openidconnect::PkceCodeVerifier::new(
                pending.pkce_verifier.clone(),
            ))
            .request_async(http_client.as_ref())
            .await
            .map_err(|e| ApiError::BadRequest(format!("token exchange failed: {e}")))?;

        let id_token = token_response
            .id_token()
            .ok_or_else(|| ApiError::BadRequest("missing id_token".into()))?;

        let verifier = client.id_token_verifier();
        let claims = id_token
            .claims(&verifier, &Nonce::new(pending.nonce.clone()))
            .map_err(|e| ApiError::BadRequest(format!("invalid id_token: {e}")))?;

        let expected_issuer = IssuerUrl::new(oidc.settings.issuer.clone())
            .map_err(|e| ApiError::Internal(format!("invalid configured issuer: {e}")))?;
        if claims.issuer() != &expected_issuer {
            return Err(ApiError::Forbidden("issuer mismatch".into()));
        }

        let sub = claims.subject().to_string();
        let email = claims.email().map(|m| m.to_string());
        let email_verified = claims.email_verified().unwrap_or(false);
        (oidc.settings.issuer.clone(), sub, email, email_verified)
    };

    // Which workspace this sign-in lands in, and whether the console should
    // offer the chooser afterwards.
    let (workspace_id, choose) = match pending.target {
        OidcPendingTarget::Workspace(id) => {
            // Login no longer checks the id, so this is where an unknown one is
            // refused, in the same words as a workspace the person has no
            // member in.
            match state.store.get_workspace(id).await {
                Ok(_) => {}
                Err(maidan_store::StoreError::NotFound) => {
                    return Err(ApiError::Forbidden(NOT_PROVISIONED.into()))
                }
                Err(err) => return Err(err.into()),
            }
            (id, false)
        }
        OidcPendingTarget::FrontDoor => {
            // The identity's own workspaces only: those where this issuer and
            // subject already have an identity row. Two rows are enough to know
            // whether there is a choice to offer.
            let listed = state
                .store
                .list_subject_workspaces(&issuer, &subject, 2)
                .await?;
            match listed.first() {
                Some(latest) => (latest.workspace_id, listed.len() > 1),
                None => {
                    // Signed in at the provider, but a member nowhere here. No
                    // session: the console says so.
                    let location = with_hint(
                        safe_return_to(pending.return_to.as_deref()),
                        NO_WORKSPACE_HINT,
                    );
                    return Ok(Redirect::temporary(&location).into_response());
                }
            }
        }
    };
    // A front-door sign-in never links by email and never provisions: it can
    // only reach a workspace where the identity already has a row.
    let front_door = pending.target == OidcPendingTarget::FrontDoor;

    let (member_id, how) = resolve_member_for_login(
        state.store.as_ref(),
        workspace_id,
        &issuer,
        &subject,
        email.as_deref(),
        email_verified,
        oidc.settings.auto_provision && !front_door,
        oidc.settings.link_email && !front_door,
    )
    .await?;
    // The provider still answers for a member it deactivated through SCIM,
    // so the deactivation is enforced here, before a session exists.
    if state
        .store
        .get_scim_user(member_id)
        .await?
        .is_some_and(|user| !user.active)
    {
        return Err(ApiError::Forbidden(
            "this member has been deactivated in this workspace".into(),
        ));
    }

    let identity = touch_identity(
        state.store.as_ref(),
        workspace_id,
        &issuer,
        &subject,
        member_id,
        email.as_deref(),
    )
    .await?;

    // A session is a credential: signing in is recorded in the session's own
    // transaction (D-A), with the member as actor.
    let session = state
        .store
        .create_session_audited(
            NewMaidanSession {
                workspace_id,
                member_id,
                api_token_id: None,
                oidc_identity_id: Some(identity.id),
                expires_at: Utc::now() + Duration::seconds(oidc.settings.session_ttl_secs as i64),
            },
            Box::new(move |session| NewAuditEvent {
                scope: AuditScope::Workspace(session.workspace_id),
                actor_id: Some(session.member_id),
                action: SESSION_CREATE.into(),
                target_kind: Some("member".into()),
                target_id: Some(session.member_id.0),
                metadata: serde_json::json!({
                    "workspace_id": session.workspace_id.0,
                    "issuer": issuer,
                    "member": how.as_str(),
                    "front_door": front_door,
                    "expires_at": session.expires_at,
                }),
            }),
        )
        .await?;

    // This browser's previous session, if any, ends now that the new one
    // exists: switching workspaces signs in again, and the old session is not
    // left live beside the new one (Hosted Console, "Switching"). Ending it is
    // recorded in its own transaction; one already gone has nothing to end,
    // and a failed end is reported rather than shown as a clean switch.
    if let Some(previous) = parse_session_cookie(&headers_in, oidc.session_secret.as_ref())
        .filter(|previous| *previous != session.id)
    {
        let ended = state
            .store
            .delete_session_audited(
                previous,
                Box::new(|ended| NewAuditEvent {
                    scope: AuditScope::Workspace(ended.workspace_id),
                    actor_id: Some(ended.member_id),
                    action: SESSION_DELETE.into(),
                    target_kind: Some("member".into()),
                    target_id: Some(ended.member_id.0),
                    metadata: serde_json::json!({
                        "workspace_id": ended.workspace_id.0,
                        "reason": "switched",
                    }),
                }),
            )
            .await;
        match ended {
            Ok(_) | Err(maidan_store::StoreError::NotFound) => {}
            Err(err) => return Err(err.into()),
        }
    }

    let mut headers = HeaderMap::new();
    set_session_cookie(
        &mut headers,
        session.id,
        oidc.settings.session_ttl_secs,
        oidc.settings.cookie_secure,
        oidc.session_secret.as_ref(),
    )
    .map_err(|e| ApiError::Internal(e.to_string()))?;

    let mut location = safe_return_to(pending.return_to.as_deref());
    if choose {
        location = with_hint(location, CHOOSE_WORKSPACE_HINT);
    }
    if oidc.settings.auto_mint {
        let has_admin = state
            .store
            .workspace_has_active_capability(workspace_id, TOKEN_ADMIN)
            .await?;
        if !has_admin {
            location = with_auto_mint_hint(location);
        }
    }
    let mut response = Redirect::temporary(&location).into_response();
    response.headers_mut().extend(headers);
    Ok(response)
}

/// End the browser session, whether OIDC or a token's, and clear its cookie.
/// Only an OIDC session goes on to the identity provider's end-session page;
/// a token's session was never signed in there.
pub async fn logout(
    State(state): State<AppState>,
    headers_in: HeaderMap,
) -> Result<Response, ApiError> {
    let settings = state
        .browser_sessions()
        .ok_or_else(|| ApiError::Forbidden("browser sessions are not configured".into()))?;

    // Ending a session is recorded in its own transaction (D-A). A session
    // already gone has nothing to end; any other failure leaves it valid, so
    // the caller is told rather than shown a sign-out that did not happen.
    // A token's session was never signed in at the identity provider.
    let mut from_token = false;
    if let Some(session_id) = parse_session_cookie(&headers_in, &settings.secret) {
        if let Ok(session) = state.store.get_session(session_id).await {
            from_token = session.api_token_id.is_some();
        }
        let ended = state
            .store
            .delete_session_audited(
                session_id,
                Box::new(|session| NewAuditEvent {
                    scope: AuditScope::Workspace(session.workspace_id),
                    actor_id: Some(session.member_id),
                    action: SESSION_DELETE.into(),
                    target_kind: Some("member".into()),
                    target_id: Some(session.member_id.0),
                    metadata: serde_json::json!({ "workspace_id": session.workspace_id.0 }),
                }),
            )
            .await;
        match ended {
            Ok(_) | Err(maidan_store::StoreError::NotFound) => {}
            Err(err) => return Err(err.into()),
        }
    }

    let mut headers = HeaderMap::new();
    clear_session_cookie(&mut headers, settings.cookie_secure)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let idp_logout = state
        .oidc
        .as_ref()
        .filter(|_| !from_token)
        .and_then(|oidc| {
            Some((
                oidc.end_session_url.as_ref()?,
                oidc.logout_client_id.as_ref()?,
                oidc,
            ))
        });
    let location = if let Some((end, client_id, oidc)) = idp_logout {
        let mut logout = LogoutRequest::from(end.clone()).set_client_id(client_id.clone());
        if let Some(uri) = &oidc.settings.post_logout_redirect_uri {
            let redirect = PostLogoutRedirectUrl::new(uri.clone()).map_err(|e| {
                ApiError::Internal(format!("invalid post-logout redirect URI: {e}"))
            })?;
            logout = logout.set_post_logout_redirect_uri(redirect);
        }
        logout.http_get_url().to_string()
    } else {
        "/ui/".to_string()
    };

    // 303, not 307: the form POST that signed out must become a GET of the
    // page, and a 307 made the browser POST to `/ui/` (a 405).
    let mut response = Redirect::to(&location).into_response();
    response.headers_mut().extend(headers);
    Ok(response)
}

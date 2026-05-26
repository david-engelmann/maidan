use axum::{
    extract::{Query, State},
    http::HeaderMap,
    response::{IntoResponse, Redirect, Response},
};
use chrono::{Duration, Utc};
use maidan_types::{NewMaidanSession, NewOidcPendingAuth, WorkspaceId};
use openidconnect::{
    core::CoreAuthenticationFlow, AuthorizationCode, IssuerUrl, LogoutRequest, Nonce,
    PkceCodeChallenge, PkceCodeVerifier, PostLogoutRedirectUrl, Scope, TokenResponse,
};
use rand::RngCore;

use crate::dto::{OidcCallbackQuery, OidcLoginQuery};
use crate::error::ApiError;
use crate::oidc::member::{resolve_member_for_login, touch_identity};
use crate::session::{clear_session_cookie, parse_session_cookie, set_session_cookie};
use crate::state::AppState;

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

pub async fn login(
    State(state): State<AppState>,
    Query(q): Query<OidcLoginQuery>,
) -> Result<Response, ApiError> {
    let oidc = state
        .oidc
        .as_ref()
        .ok_or_else(|| ApiError::Forbidden("OIDC is not enabled".into()))?;
    let workspace_id = WorkspaceId(q.workspace_id);
    state.store.get_workspace(workspace_id).await?;

    let state_token = random_token();
    let nonce = random_token();
    let pkce_verifier = PkceCodeVerifier::new(random_token());
    let pkce_secret = pkce_verifier.secret().to_string();
    let expires_at = Utc::now() + Duration::seconds(oidc.settings.pending_ttl_secs as i64);

    state
        .store
        .insert_oidc_pending(NewOidcPendingAuth {
            state: state_token.clone(),
            workspace_id,
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
    Query(q): Query<OidcCallbackQuery>,
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

    let member_id = resolve_member_for_login(
        state.store.as_ref(),
        pending.workspace_id,
        &issuer,
        &subject,
        email.as_deref(),
        email_verified,
        oidc.settings.auto_provision,
        oidc.settings.link_email,
    )
    .await?;

    touch_identity(
        state.store.as_ref(),
        pending.workspace_id,
        &issuer,
        &subject,
        member_id,
        email.as_deref(),
    )
    .await?;

    let session = state
        .store
        .create_session(NewMaidanSession {
            workspace_id: pending.workspace_id,
            member_id,
            csrf_secret: random_token(),
            expires_at: Utc::now() + Duration::seconds(oidc.settings.session_ttl_secs as i64),
        })
        .await?;

    let mut headers = HeaderMap::new();
    set_session_cookie(
        &mut headers,
        session.id,
        oidc.settings.session_ttl_secs,
        oidc.settings.cookie_secure,
        oidc.session_secret.as_ref(),
    )
    .map_err(|e| ApiError::Internal(e.to_string()))?;

    let location = safe_return_to(pending.return_to.as_deref());
    let mut response = Redirect::temporary(&location).into_response();
    response.headers_mut().extend(headers);
    Ok(response)
}

pub async fn logout(
    State(state): State<AppState>,
    headers_in: HeaderMap,
) -> Result<Response, ApiError> {
    let oidc = state
        .oidc
        .as_ref()
        .ok_or_else(|| ApiError::Forbidden("OIDC is not enabled".into()))?;

    if let Some(session_id) = parse_session_cookie(&headers_in, oidc.session_secret.as_ref()) {
        let _ = state.store.delete_session(session_id).await;
    }

    let mut headers = HeaderMap::new();
    clear_session_cookie(&mut headers, oidc.settings.cookie_secure)
        .map_err(|e| ApiError::Internal(e.to_string()))?;

    let location =
        if let (Some(end), Some(client_id)) = (&oidc.end_session_url, &oidc.logout_client_id) {
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

    let mut response = Redirect::temporary(&location).into_response();
    response.headers_mut().extend(headers);
    Ok(response)
}

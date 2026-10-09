use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Extension, Json,
};
use chrono::{DateTime, Duration, Utc};
use maidan_auth::{
    capability::{self, WORKSPACE_READ},
    hash_secret, AuthContext, TokenSecret, TOKEN_ADMIN,
};
use maidan_types::{AuditScope, MemberId, NewApiToken, NewAuditEvent, NewMaidanSession};

use crate::auth::bearer_from_headers;
use crate::dto::{MintApiTokenResponse, SessionResponse, SessionWorkspace, SessionWorkspaces};
use crate::error::ApiError;
use crate::session::{parse_session_cookie, set_session_cookie, SessionContext};
use crate::state::AppState;

async fn member_display_name(
    state: &AppState,
    member_id: MemberId,
) -> Result<Option<String>, ApiError> {
    match state.store.get_member(member_id).await {
        Ok(member) => Ok(member.display_name.and_then(|name| {
            let trimmed = name.trim();
            if trimmed.is_empty() {
                None
            } else if trimmed.len() == name.len() {
                Some(name)
            } else {
                Some(trimmed.to_string())
            }
        })),
        Err(maidan_store::StoreError::NotFound) => Ok(None),
        Err(err) => Err(err.into()),
    }
}

/// Exchange the request's bearer for a browser session holding the same
/// authority, so a page can work without keeping the token. The session
/// records the token's id and each request re-resolves it: it has exactly that
/// token's capabilities, workspace and grant, and it ends when the token is
/// revoked, rotated or expires, or at the session lifetime, whichever is first.
///
/// Creating it is an authority change (D-A): its audit row is written in the
/// same transaction. `workspace:read` is required because a browser session
/// exists to show a workspace.
pub async fn session_from_token(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    // Only a bearer is exchanged: a session that could make another would
    // renew itself past its lifetime.
    if bearer_from_headers(&headers).is_none() {
        return Err(ApiError::Unauthorized);
    }
    crate::routes::cap(&auth, WORKSPACE_READ)?;
    let settings = state.browser_sessions().ok_or(ApiError::NotFound)?;
    let token_id = auth.token_id.ok_or(ApiError::Unauthorized)?;
    let token = state.store.get_api_token(token_id).await?;

    let now = Utc::now();
    let lifetime_end = i64::try_from(settings.ttl_secs)
        .ok()
        .and_then(Duration::try_seconds)
        .and_then(|ttl| now.checked_add_signed(ttl))
        .unwrap_or(DateTime::<Utc>::MAX_UTC);
    let expires_at = token
        .expires_at
        .map_or(lifetime_end, |end| end.min(lifetime_end));

    // This browser's previous session, if any, is replaced rather than left
    // live beside the new one.
    if let Some(previous) = parse_session_cookie(&headers, &settings.secret) {
        let ended = state
            .store
            .delete_session_audited(
                previous,
                Box::new(|session| NewAuditEvent {
                    scope: AuditScope::Workspace(session.workspace_id),
                    actor_id: Some(session.member_id),
                    action: "session.delete".into(),
                    target_kind: Some("member".into()),
                    target_id: Some(session.member_id.0),
                    metadata: serde_json::json!({
                        "workspace_id": session.workspace_id.0,
                        "reason": "replaced",
                    }),
                }),
            )
            .await;
        match ended {
            Ok(_) | Err(maidan_store::StoreError::NotFound) => {}
            Err(err) => return Err(err.into()),
        }
    }

    let actor = auth.actor_id;
    let grant = auth.delegation_grant_id;
    let session = state
        .store
        .create_session_audited(
            NewMaidanSession {
                workspace_id: auth.workspace_id,
                member_id: auth.member_id,
                api_token_id: Some(token_id),
                oidc_identity_id: None,
                expires_at,
            },
            Box::new(move |session| NewAuditEvent {
                actor_id: Some(actor),
                scope: AuditScope::Workspace(session.workspace_id),
                action: "session.from_token".into(),
                target_kind: Some("api_token".into()),
                target_id: Some(token_id.0),
                metadata: serde_json::json!({
                    "workspace_id": session.workspace_id.0,
                    "subject_member_id": session.member_id.0,
                    "grant_id": grant.map(|g| g.0),
                    "expires_at": session.expires_at,
                }),
            }),
        )
        .await?;

    let max_age = u64::try_from((session.expires_at - now).num_seconds()).unwrap_or(0);
    let mut response = (
        StatusCode::CREATED,
        Json(SessionResponse {
            member_id: session.member_id,
            workspace_id: session.workspace_id,
            expires_at: session.expires_at,
            token_id: session.api_token_id,
            display_name: member_display_name(&state, session.member_id).await?,
        }),
    )
        .into_response();
    set_session_cookie(
        response.headers_mut(),
        session.id,
        max_age,
        settings.cookie_secure,
        &settings.secret,
    )
    .map_err(|e| ApiError::Internal(e.to_string()))?;
    Ok(response)
}

pub async fn mint_first_admin_token(
    State(state): State<AppState>,
    Extension(ctx): Extension<SessionContext>,
) -> Result<(StatusCode, Json<MintApiTokenResponse>), ApiError> {
    let oidc = state
        .oidc
        .as_ref()
        .ok_or_else(|| ApiError::Forbidden("OIDC is not enabled".into()))?;
    // The first-admin mint is for a person who signed in, not a token's
    // holder: a narrowed or borrowed token's session must not become admin.
    if ctx.token.is_some() {
        return Err(ApiError::Forbidden(
            "a session made from a token cannot mint the first admin token".into(),
        ));
    }
    if !oidc.settings.first_admin_mint {
        return Err(ApiError::Forbidden(
            "first-admin session mint is disabled (MAIDAN_OIDC_FIRST_ADMIN)".into(),
        ));
    }
    if state
        .store
        .workspace_has_active_capability(ctx.workspace_id, TOKEN_ADMIN)
        .await?
    {
        return Err(ApiError::Forbidden(
            "workspace already has a token:admin holder".into(),
        ));
    }

    let mut capabilities = capability::default_minted();
    capabilities.push(TOKEN_ADMIN.to_string());
    let secret = TokenSecret::generate();
    let actor = ctx.member_id;
    let record = state
        .store
        .create_api_token_audited(
            NewApiToken {
                workspace_id: ctx.workspace_id,
                member_id: ctx.member_id,
                app_installation_id: None,
                token_hash: hash_secret(secret.as_str()),
                label: Some("oidc-first-admin".into()),
                capabilities,
                expires_at: None,
            },
            Box::new(move |record| NewAuditEvent {
                scope: AuditScope::Workspace(record.workspace_id),
                actor_id: Some(actor),
                action: "token.mint".into(),
                target_kind: Some("api_token".into()),
                target_id: Some(record.id.0),
                metadata: serde_json::json!({
                    "workspace_id": record.workspace_id.0,
                    "subject_member_id": record.member_id.0,
                    "capabilities": record.capabilities.clone(),
                    "source": "oidc-first-admin",
                }),
            }),
        )
        .await?;

    Ok((
        StatusCode::CREATED,
        Json(MintApiTokenResponse {
            id: record.id,
            secret: secret.as_str().to_string(),
            workspace_id: record.workspace_id,
            member_id: record.member_id,
            capabilities: record.capabilities,
            expires_at: record.expires_at,
            quotas: vec![],
        }),
    ))
}

pub async fn get_session(
    State(state): State<AppState>,
    Extension(ctx): Extension<SessionContext>,
) -> Result<Json<SessionResponse>, ApiError> {
    let session = state.store.get_session(ctx.session_id).await?;
    if session.expires_at < Utc::now() {
        let _ = state.store.delete_expired_session(session.id).await;
        return Err(ApiError::Unauthorized);
    }
    Ok(Json(SessionResponse {
        member_id: session.member_id,
        workspace_id: session.workspace_id,
        expires_at: session.expires_at,
        token_id: session.api_token_id,
        display_name: member_display_name(&state, session.member_id).await?,
    }))
}

/// The most workspaces `GET /auth/session/workspaces` lists. The console
/// searches this list by name; the route has no search of its own.
pub const SESSION_WORKSPACES_LIMIT: i64 = 200;

/// The workspaces this browser session can switch to (Open Work Next 6,
/// `docs/Hosted Console.md`).
///
/// An OIDC session lists every workspace where the identity it signed in with
/// (the same issuer and subject) has an identity row, its own first. Nothing
/// the client sends picks the identity. A session made from a token, or one
/// from before the identity was recorded, lists only its own workspace: a
/// token proves nothing about the person behind it.
pub async fn list_session_workspaces(
    State(state): State<AppState>,
    Extension(ctx): Extension<SessionContext>,
) -> Result<Json<SessionWorkspaces>, ApiError> {
    let session = state.store.get_session(ctx.session_id).await?;
    if session.expires_at < Utc::now() {
        let _ = state.store.delete_expired_session(session.id).await;
        return Err(ApiError::Unauthorized);
    }
    let listed = match session
        .oidc_identity_id
        .filter(|_| session.api_token_id.is_none())
    {
        Some(identity) => {
            state
                .store
                .list_identity_workspaces(identity, SESSION_WORKSPACES_LIMIT)
                .await?
        }
        None => Vec::new(),
    };
    let mut workspaces: Vec<SessionWorkspace> = Vec::with_capacity(listed.len().max(1));
    let current = match listed
        .iter()
        .find(|w| w.workspace_id == session.workspace_id && w.member_id == session.member_id)
    {
        Some(row) => SessionWorkspace {
            workspace_id: row.workspace_id,
            name: row.workspace_name.clone(),
            member_id: row.member_id,
            handle: row.handle.clone(),
            current: true,
        },
        None => {
            let workspace = state.store.get_workspace(session.workspace_id).await?;
            let member = state.store.get_member(session.member_id).await?;
            SessionWorkspace {
                workspace_id: workspace.id,
                name: workspace.name,
                member_id: member.id,
                handle: member.handle,
                current: true,
            }
        }
    };
    workspaces.push(current);
    workspaces.extend(
        listed
            .into_iter()
            .filter(|w| w.workspace_id != session.workspace_id)
            .map(|w| SessionWorkspace {
                workspace_id: w.workspace_id,
                name: w.workspace_name,
                member_id: w.member_id,
                handle: w.handle,
                current: false,
            }),
    );
    Ok(Json(SessionWorkspaces { workspaces }))
}

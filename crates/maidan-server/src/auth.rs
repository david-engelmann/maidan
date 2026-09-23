//! Bearer authentication middleware and helpers.

use axum::{
    extract::{Request, State},
    http::header,
    middleware::Next,
    response::{IntoResponse, Response},
};
use maidan_auth::{
    capability::{EVENT_SUBSCRIBE, MESSAGE_POST, SEARCH_QUERY, WORKSPACE_READ, WORKSPACE_WRITE},
    record_delegated_authorization, resolve_bearer, resolve_peer_bearer, AuthContext,
    AuthorizationDecision, AuthorizationOutcome, AuthorizationSurface,
};

use crate::error::ApiError;
use crate::federation::PeerContext;
use crate::session::load_session;
use crate::state::AppState;

const TEST_MEMBER_HEADER: &str = "maidan-test-member-id";

async fn auth_disabled_context(state: &AppState, headers: &axum::http::HeaderMap) -> AuthContext {
    if state.test_identity_header {
        if let Some(member_id) = headers
            .get(TEST_MEMBER_HEADER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<uuid::Uuid>().ok())
            .map(maidan_types::MemberId)
        {
            if let Ok(member) = state.store.get_member(member_id).await {
                return AuthContext::from_session(
                    member_id,
                    member.workspace_id,
                    maidan_auth::capability::all(),
                );
            }
        }
    }
    AuthContext::bypass()
}

/// Whether bearer auth is actually disabled. Fail-closed: `AUTH_DISABLED` takes
/// effect only when the operator has explicitly acknowledged it via
/// `MAIDAN_ALLOW_INSECURE_NO_AUTH` and the deployment is not production. This
/// mirrors the startup validation in [`crate::config`] as defense-in-depth, so
/// the code path that flips on `AuthContext::bypass()` can never do so from a
/// stray `AUTH_DISABLED=1` alone.
pub fn auth_disabled_from_env() -> bool {
    crate::config::auth_disabled_requested()
        && crate::config::insecure_no_auth_acknowledged()
        && !crate::config::is_production()
}

pub async fn middleware(State(state): State<AppState>, mut req: Request, next: Next) -> Response {
    if state.auth_disabled {
        let auth = auth_disabled_context(&state, req.headers()).await;
        req.extensions_mut().insert(auth);
        return next.run(req).await;
    }

    let bearer = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(parse_bearer);

    let Some(secret) = bearer else {
        record_authentication_denial(req.uri().path());
        return ApiError::Unauthorized.into_response();
    };

    match resolve_bearer(state.store.as_ref(), secret).await {
        Ok(ctx) => {
            let workspace_id = ctx.workspace_id;
            let method = req.method().clone();
            let path = req.uri().path().to_owned();
            req.extensions_mut().insert(ctx.clone());
            let response = next.run(req).await;
            if !path.starts_with("/mcp") {
                let outcome = if matches!(response.status().as_u16(), 401 | 403 | 404) {
                    AuthorizationOutcome::Denied
                } else {
                    AuthorizationOutcome::Allowed
                };
                record_delegated_authorization(
                    state.store.as_ref(),
                    &ctx,
                    AuthorizationSurface::Rest,
                    &format!("{method} {path}"),
                    outcome,
                )
                .await;
            }
            tag_room(response, workspace_id)
        }
        Err(_) => match resolve_peer_bearer(state.store.as_ref(), secret).await {
            Ok(peer) => {
                let workspace_id = peer.workspace_id;
                req.extensions_mut().insert(PeerContext(peer));
                tag_room(next.run(req).await, workspace_id)
            }
            Err(_) => {
                record_authentication_denial(req.uri().path());
                ApiError::Unauthorized.into_response()
            }
        },
    }
}

fn record_authentication_denial(path: &str) {
    let surface = if path == "/mcp" || path.starts_with("/mcp/") {
        AuthorizationSurface::Mcp
    } else {
        AuthorizationSurface::Rest
    };
    AuthorizationDecision::authentication_denied(surface).record();
}

/// Attach the resolved room to the response for the Room-LSN layer, which is
/// applied outside every auth layer and so cannot resolve the caller's
/// workspace itself.
fn tag_room(mut resp: Response, workspace_id: maidan_types::WorkspaceId) -> Response {
    resp.extensions_mut()
        .insert(crate::room_lsn::RoomScope(workspace_id));
    resp
}

pub fn parse_bearer(header_value: &str) -> Option<&str> {
    let rest = header_value.strip_prefix("Bearer ")?;
    let token = rest.trim();
    if token.is_empty() {
        None
    } else {
        Some(token)
    }
}

pub fn bearer_from_headers(headers: &axum::http::HeaderMap) -> Option<&str> {
    headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(parse_bearer)
}

/// Accept bearer token or valid `maidan_session` cookie (for UI / operator routes).
pub async fn session_or_bearer_middleware(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Response {
    if state.auth_disabled {
        let auth = auth_disabled_context(&state, req.headers()).await;
        req.extensions_mut().insert(auth);
        return next.run(req).await;
    }

    if let Some(secret) = bearer_from_headers(req.headers()) {
        if let Ok(ctx) = resolve_bearer(state.store.as_ref(), secret).await {
            let workspace_id = ctx.workspace_id;
            req.extensions_mut().insert(ctx);
            return tag_room(next.run(req).await, workspace_id);
        }
    }

    match load_session(&state, req.headers()).await {
        Ok(session) => {
            let ctx = AuthContext::from_session(
                session.member_id,
                session.workspace_id,
                vec![
                    WORKSPACE_READ.into(),
                    EVENT_SUBSCRIBE.into(),
                    SEARCH_QUERY.into(),
                ],
            );
            let workspace_id = session.workspace_id;
            req.extensions_mut().insert(session);
            req.extensions_mut().insert(ctx);
            tag_room(next.run(req).await, workspace_id)
        }
        Err(err) => {
            record_authentication_denial(req.uri().path());
            err.into_response()
        }
    }
}

/// Browser session or bearer for `/ui/api` writes (channel browser).
pub async fn ui_session_or_bearer_middleware(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Response {
    if state.auth_disabled {
        let auth = auth_disabled_context(&state, req.headers()).await;
        req.extensions_mut().insert(auth);
        return next.run(req).await;
    }

    if let Some(secret) = bearer_from_headers(req.headers()) {
        if let Ok(ctx) = resolve_bearer(state.store.as_ref(), secret).await {
            let workspace_id = ctx.workspace_id;
            req.extensions_mut().insert(ctx);
            return tag_room(next.run(req).await, workspace_id);
        }
    }

    match load_session(&state, req.headers()).await {
        Ok(session) => {
            let ctx = AuthContext::from_session(
                session.member_id,
                session.workspace_id,
                vec![
                    WORKSPACE_READ.into(),
                    WORKSPACE_WRITE.into(),
                    MESSAGE_POST.into(),
                    EVENT_SUBSCRIBE.into(),
                    SEARCH_QUERY.into(),
                ],
            );
            let workspace_id = session.workspace_id;
            req.extensions_mut().insert(session);
            req.extensions_mut().insert(ctx);
            tag_room(next.run(req).await, workspace_id)
        }
        Err(err) => {
            record_authentication_denial(req.uri().path());
            err.into_response()
        }
    }
}

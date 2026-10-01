//! Bearer authentication middleware and helpers.

use axum::{
    extract::{MatchedPath, Request, State},
    http::{header, Method},
    middleware::Next,
    response::{IntoResponse, Response},
};
use maidan_auth::{
    capability::{EVENT_SUBSCRIBE, MESSAGE_POST, SEARCH_QUERY, WORKSPACE_READ, WORKSPACE_WRITE},
    record_delegated_authorization, resolve_bearer, resolve_peer_bearer, AuthContext,
    AuthorizationDecision, AuthorizationOutcome, AuthorizationSurface,
};

use maidan_types::{AuditScope, NewAuditEvent};

use crate::error::ApiError;
use crate::federation::PeerContext;
use crate::session::{check_request_origin, load_session, SessionContext};
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

/// Run the rest of the request as its authenticated caller, so that everything
/// it writes records who acted and on whose behalf. Every path out of the auth
/// middlewares goes through here; a bypassed request carries no principal and
/// records none.
///
/// It also guarantees that a successful change leaves a record. Most changes
/// record themselves — an event, an audit row — but not all of them do, and a
/// rule each handler must remember is one the next handler forgets. So if a
/// mutating request succeeds and nothing inside it wrote an attributed record,
/// this writes one: the operation, its concrete path, and who acted for whom.
/// MCP and A2A are exempt here because one endpoint serves many methods, some
/// of them reads, and a JSON-RPC error still answers 200: each records per
/// call instead (`McpServer::tools_call`, `a2a_agent::recorded`).
async fn run_as(state: &AppState, req: Request, next: Next) -> Response {
    let Some(auth) = req.extensions().get::<AuthContext>().cloned() else {
        return next.run(req).await;
    };
    let attribution = auth.attribution();
    let method = req.method().clone();
    let path = req.uri().path().to_owned();
    let operation = req
        .extensions()
        .get::<MatchedPath>()
        .map(|matched| matched.as_str().to_owned())
        .unwrap_or_else(|| path.clone());
    let (response, recorded) =
        maidan_store::attribution::with_attribution_tracked(attribution, next.run(req)).await;
    let operation = format!("{method} {operation}");
    let mutating = matches!(
        method,
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    ) && READ_ONLY_OPERATIONS
        .binary_search(&operation.as_str())
        .is_err();
    if attribution.is_some()
        && mutating
        && !recorded
        && response.status().is_success()
        && !records_per_call(&path)
    {
        maidan_store::attribution::with_attribution(
            attribution,
            crate::audit::record(
                state,
                NewAuditEvent {
                    scope: AuditScope::Workspace(auth.workspace_id),
                    actor_id: Some(auth.actor_id),
                    action: MUTATION_ACTION.into(),
                    target_kind: Some("workspace".into()),
                    target_id: Some(auth.workspace_id.0),
                    metadata: serde_json::json!({
                        "surface": "rest",
                        "operation": operation,
                        "path": path,
                        "status": response.status().as_u16(),
                    }),
                },
            ),
        )
        .await;
    }
    response
}

/// Surfaces that record each call they dispatch, so the request layer does not
/// record the request.
fn records_per_call(path: &str) -> bool {
    path.starts_with("/mcp") || path.starts_with("/a2a/")
}

/// The audit action of a change that did not record itself.
pub const MUTATION_ACTION: &str = "mutation";

/// Operations that change nothing although their method says they might, so
/// the request layer writes no `mutation` row for them. Without this, verifying
/// an export recorded a change that never happened. Listing an operation here
/// is a claim that it writes nothing: getting it wrong loses a record, so it
/// matches the non-GET `reads` in `contracts/http-operation-kinds.json`
/// (`http_operation_kinds_contract`), and `http_operation_kinds_e2e` checks
/// that each leaves the database as it found it. Sorted, for `binary_search`.
pub const READ_ONLY_OPERATIONS: &[&str] = &[
    "POST /threads/{id}/land-gate/advice",
    "POST /workspaces/export/verify",
    "POST /workspaces/{wid}/secrets/{name}/resolve",
];

pub async fn middleware(State(state): State<AppState>, mut req: Request, next: Next) -> Response {
    if state.auth_disabled {
        let auth = auth_disabled_context(&state, req.headers()).await;
        req.extensions_mut().insert(auth);
        return run_as(&state, req, next).await;
    }

    let bearer = req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(parse_bearer);

    let Some(secret) = bearer else {
        // A page that exchanged its token for a session sends no bearer.
        let session = token_session(&state, req.uri().path(), req.method(), req.headers()).await;
        return match session {
            Ok((session, ctx)) => {
                req.extensions_mut().insert(session);
                run_authorized(&state, req, next, ctx).await
            }
            Err(err) => {
                if matches!(err, ApiError::Unauthorized) {
                    record_authentication_denial(req.uri().path());
                }
                err.into_response()
            }
        };
    };

    match resolve_bearer(state.store.as_ref(), secret).await {
        Ok(ctx) => run_authorized(&state, req, next, ctx).await,
        Err(_) => match resolve_peer_bearer(state.store.as_ref(), secret).await {
            Ok(peer) => {
                let workspace_id = peer.workspace_id;
                req.extensions_mut().insert(PeerContext(peer));
                tag_room(run_as(&state, req, next).await, workspace_id)
            }
            Err(_) => {
                record_authentication_denial(req.uri().path());
                ApiError::Unauthorized.into_response()
            }
        },
    }
}

/// Run a request on the bearer tree as `ctx`, recording a delegated caller's
/// decision.
async fn run_authorized(
    state: &AppState,
    mut req: Request,
    next: Next,
    ctx: AuthContext,
) -> Response {
    let workspace_id = ctx.workspace_id;
    let method = req.method().clone();
    let path = req.uri().path().to_owned();
    req.extensions_mut().insert(ctx.clone());
    let response = run_as(state, req, next).await;
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

/// The session a bearer-tree request without a bearer rides on. Only a session
/// made from a token reaches this tree, with that token's authority: the bearer
/// routes accept exactly what the token's bearer would. An OIDC session's fixed
/// capabilities were sized for the `/ui/api` routes and stay there, and MCP is
/// an agent protocol, bearer only.
async fn token_session(
    state: &AppState,
    path: &str,
    method: &Method,
    headers: &axum::http::HeaderMap,
) -> Result<(SessionContext, AuthContext), ApiError> {
    if path.starts_with("/mcp") {
        return Err(ApiError::Unauthorized);
    }
    let session = load_session(state, headers).await?;
    let ctx = session.token.clone().ok_or(ApiError::Unauthorized)?;
    check_request_origin(method, headers)?;
    Ok((session, ctx))
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

/// What an OIDC session may do on the `/ui/api` read routes.
pub const OIDC_READ_CAPABILITIES: &[&str] = &[WORKSPACE_READ, EVENT_SUBSCRIBE, SEARCH_QUERY];

/// What an OIDC session may do on the `/ui/api` write routes.
const OIDC_WRITE_CAPABILITIES: &[&str] = &[
    WORKSPACE_READ,
    WORKSPACE_WRITE,
    MESSAGE_POST,
    EVENT_SUBSCRIBE,
    SEARCH_QUERY,
];

/// Accept bearer token or valid `maidan_session` cookie (for UI / operator routes).
pub async fn session_or_bearer_middleware(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Response {
    session_or_bearer(&state, req, next, OIDC_READ_CAPABILITIES).await
}

/// Browser session or bearer for `/ui/api` writes (channel browser).
pub async fn ui_session_or_bearer_middleware(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Response {
    session_or_bearer(&state, req, next, OIDC_WRITE_CAPABILITIES).await
}

/// A bearer, else a session: a token's session carries that token's authority,
/// an OIDC session `oidc_capabilities`. An unsafe request on a session must
/// come from this origin.
async fn session_or_bearer(
    state: &AppState,
    mut req: Request,
    next: Next,
    oidc_capabilities: &[&str],
) -> Response {
    if state.auth_disabled {
        let auth = auth_disabled_context(state, req.headers()).await;
        req.extensions_mut().insert(auth);
        return run_as(state, req, next).await;
    }

    if let Some(secret) = bearer_from_headers(req.headers()) {
        if let Ok(ctx) = resolve_bearer(state.store.as_ref(), secret).await {
            // The bearer tree records a delegated 401/403/404. These UI groups
            // are mounted beside it, so a denied token here has to go through
            // the same path or the `authorization.decision` row is never written.
            return run_authorized(state, req, next, ctx).await;
        }
    }

    match load_session(state, req.headers()).await {
        Ok(session) => {
            let ctx = session.auth_context(oidc_capabilities);
            if let Err(err) = check_request_origin(req.method(), req.headers()) {
                record_delegated_authorization(
                    state.store.as_ref(),
                    &ctx,
                    AuthorizationSurface::Rest,
                    &format!("{} {}", req.method(), req.uri().path()),
                    AuthorizationOutcome::Denied,
                )
                .await;
                return err.into_response();
            }
            req.extensions_mut().insert(session);
            run_authorized(state, req, next, ctx).await
        }
        Err(err) => {
            record_authentication_denial(req.uri().path());
            err.into_response()
        }
    }
}

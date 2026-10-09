//! MCP streamable HTTP: JSON-RPC response plus live notifications on one SSE
//! stream (`POST /mcp/streamable`), with follow-ups multiplexed onto an open
//! session.

use std::convert::Infallible;
use std::time::Duration;

use axum::response::sse::{Event, KeepAlive};
use axum::{
    extract::State,
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode},
    response::{IntoResponse, Response, Sse},
    Extension, Json,
};
use maidan_auth::{capability::WORKSPACE_READ, AuthContext};
use maidan_mcp::{JsonRpcRequest, JsonRpcResponse, McpSession, Principal};
use tokio_stream::StreamExt as _;

use crate::error::ApiError;
use crate::extract::ApiBytes;
use crate::state::AppState;

/// How often a session's notification task checks that the session is still
/// open.
const SESSION_LIVENESS_CHECK: Duration = Duration::from_secs(15);

/// Whether the client's `Accept` header permits an SSE response. Absent →
/// `true` (preserve the streaming default). MCP spec: the server may answer a
/// request with a single `application/json` body when the client accepts it.
fn accepts_event_stream(headers: &HeaderMap) -> bool {
    match headers
        .get(axum::http::header::ACCEPT)
        .and_then(|v| v.to_str().ok())
    {
        Some(accept) => accept.contains("text/event-stream") || accept.contains("*/*"),
        None => true,
    }
}

/// The `Last-Event-ID` header parsed as the session event id to resume after.
fn last_event_id(headers: &HeaderMap) -> Option<u64> {
    headers
        .get("last-event-id")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.trim().parse().ok())
}

pub async fn streamable(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    headers: HeaderMap,
    ApiBytes(body): ApiBytes,
) -> Result<Response, ApiError> {
    if !auth.bypass {
        auth.require_capability(WORKSPACE_READ)
            .map_err(|_| ApiError::Forbidden("missing workspace:read capability".into()))?;
    }
    let session_header = headers.get("mcp-session-id").and_then(|v| v.to_str().ok());
    let request = match maidan_mcp::protocol::parse_request(&body) {
        Ok(r) => r,
        Err(rejected) => return Ok(Json(JsonRpcResponse::rejected(rejected)).into_response()),
    };
    // The revision's own checks: on `2026-07-28` the version header, `_meta`,
    // the routing headers and removed methods; on earlier revisions, routing
    // headers that are sent must match the body (SEP-2243, J3.2).
    let era = match crate::mcp::admit(&headers, &request) {
        Ok(era) => era,
        Err(refused) => return Ok(*refused),
    };
    if let Err(resp) = crate::mcp_quota::enforce_mcp_quota(&state, &auth, &request).await {
        return Ok(Json(resp).into_response());
    }

    // Every anonymous caller is the same member, so a session one of them
    // opened would be open to all of them: an anonymous caller stays stateless.
    if auth.is_anonymous() {
        let notification = request.id.is_none();
        let response = state.mcp.handle(request, &auth).await;
        if notification {
            return Ok(StatusCode::ACCEPTED.into_response());
        }
        return Ok(crate::mcp::reply(era, response));
    }

    // A follow-up on an open `2024-11-05` session stays on it — if this caller
    // opened it; anyone else's session id is no session at all.
    let registry = state.mcp.streamable_sessions();
    if let Some(existing) = session_header.filter(|s| !s.is_empty()) {
        if registry.is_open_for(existing, &Principal::of(&auth)).await {
            return follow_up_on_open_session(&state, &auth, existing, request).await;
        }
    }

    // Every revision from `2025-03-26` on is stateless (Protocols.md J3.3-4): a
    // request lands cold and gets one JSON-RPC response on its own POST — we
    // never mint or require an `Mcp-Session-Id`, regardless of `Accept`, and a
    // notification is acknowledged with `202` and no body. Server-initiated
    // messages ride `GET /mcp/streamable` / `GET /mcp/stream` / WS / the
    // `wait_for_*` tools, not a POST session.
    if era == crate::mcp::Era::Current || !crate::mcp::wants_session(&headers, &request) {
        let notification = request.id.is_none();
        let response = state.mcp.handle(request, &auth).await;
        if notification {
            return Ok(StatusCode::ACCEPTED.into_response());
        }
        return Ok(crate::mcp::reply(era, response));
    }

    // Content negotiation: a client that accepts only JSON gets a single
    // response body rather than an SSE session (MCP spec allows either).
    if !accepts_event_stream(&headers) {
        let response = state.mcp.handle(request, &auth).await;
        return Ok(Json(response).into_response());
    }

    open_new_streamable_session(&state, &auth, session_header, request).await
}

async fn follow_up_on_open_session(
    state: &AppState,
    auth: &AuthContext,
    session_id: &str,
    request: JsonRpcRequest,
) -> Result<Response, ApiError> {
    let session = McpSession::Streamable(session_id.to_string());
    let response = state.mcp.handle_in(request, auth, &session).await;
    // Mux onto the open SSE leg (202). If that leg has since dropped — the
    // session survives it now, for reconnect — the response was still logged
    // for replay; answer it inline (200) rather than failing.
    let mut resp = if push_response(state, session_id, &response).await.is_ok() {
        StatusCode::ACCEPTED.into_response()
    } else {
        Json(response).into_response()
    };
    attach_session_header(&mut resp, session_id);
    Ok(resp)
}

async fn open_new_streamable_session(
    state: &AppState,
    auth: &AuthContext,
    session_header: Option<&str>,
    request: JsonRpcRequest,
) -> Result<Response, ApiError> {
    let session_id = state
        .mcp
        .touch_streamable_session(session_header, auth)
        .await;
    let registry = state.mcp.streamable_sessions();
    let sse_rx = registry.open(session_id.clone(), Principal::of(auth)).await;
    let session = McpSession::Streamable(session_id.clone());

    // Listen before handling, so an update the first request causes is not
    // missed. The session's notifications ride its own stream until it closes.
    let mut listener = state.mcp.listen(auth, session.clone()).await;
    let response = state.mcp.handle_in(request, auth, &session).await;
    push_response(state, &session_id, &response).await?;

    let registry_bg = registry.clone();
    let session_bg = session_id.clone();
    let owner = Principal::of(auth);
    let credential = crate::stream_guard::StreamCredential::from_auth(auth);
    let store_bg = state.store.clone();
    let mcp_bg = state.mcp.clone();
    let auth_bg = auth.clone();
    maidan_store::attribution::spawn(async move {
        // A closed or expired session gets no more notifications, so it is
        // noticed by checking, not by a failed push that never comes.
        let mut liveness = tokio::time::interval(SESSION_LIVENESS_CHECK);
        loop {
            tokio::select! {
                next = listener.recv() => {
                    let Some(notification) = next else { break };
                    if let Ok(data) = serde_json::to_string(&notification) {
                        if !registry_bg.push(&session_bg, data).await {
                            break;
                        }
                    }
                }
                _ = liveness.tick() => {
                    if !registry_bg.is_open_for(&session_bg, &owner).await {
                        break;
                    }
                    // A deactivated member or revoked token closes the
                    // session, and with it the subscriptions made in it.
                    if credential.recheck(store_bg.as_ref()).await.is_err() {
                        mcp_bg.close_streamable_session(&session_bg, &auth_bg).await;
                        break;
                    }
                }
            }
        }
    });

    // Each frame carries its `id:` so a dropped client can resume with
    // `Last-Event-ID`. The session is *not* closed when this stream ends — it
    // stays open (with its replay log) for reconnect until TTL or DELETE.
    let stream = futures::stream::unfold(sse_rx, move |mut rx| async move {
        rx.recv().await.map(|(event_id, data)| {
            (
                Ok::<Event, Infallible>(Event::default().id(event_id.to_string()).data(data)),
                rx,
            )
        })
    });
    let stream = crate::stream_guard::guard(state.store.clone(), auth, stream);

    let mut resp = Sse::new(stream)
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(15))
                .text("ping"),
        )
        .into_response();
    attach_session_header(&mut resp, &session_id);
    Ok(resp)
}

async fn push_response(
    state: &AppState,
    session_id: &str,
    response: &JsonRpcResponse,
) -> Result<(), ApiError> {
    let json = serde_json::to_string(response).map_err(|e| ApiError::Internal(e.to_string()))?;
    if !state.mcp.streamable_sessions().push(session_id, json).await {
        return Err(ApiError::Internal("streamable session closed".into()));
    }
    Ok(())
}

/// Close an open streamable session (`DELETE /mcp/streamable`), and with it
/// every resource subscription made in it. Another caller's session id is
/// left alone.
pub async fn close_session(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    if !auth.bypass {
        auth.require_capability(WORKSPACE_READ)
            .map_err(|_| ApiError::Forbidden("missing workspace:read capability".into()))?;
    }
    let Some(session_id) = headers
        .get("mcp-session-id")
        .and_then(|v| v.to_str().ok())
        .filter(|s| !s.is_empty())
    else {
        return Err(ApiError::BadRequest("missing Mcp-Session-Id header".into()));
    };
    state.mcp.close_streamable_session(session_id, &auth).await;
    Ok(StatusCode::NO_CONTENT)
}

/// Server→client SSE stream for a streamable session (`GET /mcp/streamable`).
/// Delivers unsolicited server notifications (e.g. resource updates) per the
/// MCP spec's server-initiated GET stream: with an open `Mcp-Session-Id` this
/// caller owns, what it subscribed to in that session (touched and echoed);
/// otherwise what it subscribed to statelessly.
pub async fn stream_get(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    if !auth.bypass {
        auth.require_capability(WORKSPACE_READ)
            .map_err(|_| ApiError::Forbidden("missing workspace:read capability".into()))?;
    }
    crate::mcp::validate_protocol_version(&headers)?;

    let registry = state.mcp.streamable_sessions();
    let owner = Principal::of(&auth);
    let session_id = match headers.get("mcp-session-id").and_then(|v| v.to_str().ok()) {
        Some(id) if !id.is_empty() && registry.is_open_for(id, &owner).await => {
            registry.touch(id).await;
            Some(id.to_string())
        }
        _ => None,
    };

    // Resumability: with an open session and a `Last-Event-ID`, replay the
    // retained frames after that id before the live stream.
    let replay_frames = match (&session_id, last_event_id(&headers)) {
        (Some(id), Some(after)) => registry.replay_after(id, after).await,
        _ => Vec::new(),
    };
    let replay = futures::stream::iter(replay_frames.into_iter().map(|(event_id, data)| {
        Ok::<Event, Infallible>(Event::default().id(event_id.to_string()).data(data))
    }));

    let session = session_id
        .clone()
        .map_or(McpSession::Stateless, McpSession::Streamable);
    let notifications = state
        .mcp
        .listen(&auth, session)
        .await
        .into_stream()
        .filter_map(|notification| {
            serde_json::to_string(&notification)
                .ok()
                .map(|data| Ok::<Event, Infallible>(Event::default().data(data)))
        });

    let stream =
        crate::stream_guard::guard(state.store.clone(), &auth, replay.chain(notifications));

    let mut resp = Sse::new(stream)
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(15))
                .text("ping"),
        )
        .into_response();
    if let Some(id) = session_id {
        attach_session_header(&mut resp, &id);
    }
    Ok(resp)
}

fn attach_session_header(resp: &mut Response, session_id: &str) {
    if let Ok(value) = HeaderValue::from_str(session_id) {
        resp.headers_mut()
            .insert(HeaderName::from_static("mcp-session-id"), value);
    }
}

#[cfg(test)]
mod tests {
    use axum::http::StatusCode;

    use super::*;

    #[test]
    fn attach_session_header_sets_mcp_session_id() {
        let mut resp = StatusCode::OK.into_response();
        attach_session_header(&mut resp, "sess-123");
        assert_eq!(resp.headers().get("mcp-session-id").unwrap(), "sess-123");
    }
}

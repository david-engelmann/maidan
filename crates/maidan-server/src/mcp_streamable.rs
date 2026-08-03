//! MCP streamable HTTP: JSON-RPC response plus live notifications on one SSE stream
//! (`POST /mcp/streamable`). Cluster 27; follow-up mux on open session Cluster 78.

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
use maidan_mcp::{JsonRpcRequest, JsonRpcResponse};
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt as _;

use crate::error::ApiError;
use crate::state::AppState;

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

pub async fn streamable(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Result<Response, ApiError> {
    if !auth.bypass {
        auth.require_capability(WORKSPACE_READ)
            .map_err(|_| ApiError::Forbidden("missing workspace:read capability".into()))?;
    }
    crate::mcp::validate_protocol_version(&headers)?;

    let request: JsonRpcRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(_) => return Ok(Json(JsonRpcResponse::parse_error()).into_response()),
    };
    if let Err(resp) = crate::mcp_quota::enforce_mcp_quota(&state, &auth, &request).await {
        return Ok(Json(resp).into_response());
    }

    // Content negotiation: a client that accepts only JSON gets a single
    // response body rather than an SSE session (MCP spec allows either).
    if !accepts_event_stream(&headers) {
        let response = state.mcp.handle(request, &auth).await;
        return Ok(Json(response).into_response());
    }

    let session_header = headers.get("mcp-session-id").and_then(|v| v.to_str().ok());
    let registry = state.mcp.streamable_sessions();

    if let Some(existing) = session_header.filter(|s| !s.is_empty()) {
        if registry.is_open(existing).await {
            return follow_up_on_open_session(&state, &auth, existing, request).await;
        }
    }

    open_new_streamable_session(&state, &auth, session_header, request).await
}

async fn follow_up_on_open_session(
    state: &AppState,
    auth: &AuthContext,
    session_id: &str,
    request: JsonRpcRequest,
) -> Result<Response, ApiError> {
    let response = state.mcp.handle(request, auth).await;
    push_response_and_notifications(state, session_id, &response).await?;
    let mut resp = StatusCode::ACCEPTED.into_response();
    attach_session_header(&mut resp, session_id);
    Ok(resp)
}

async fn open_new_streamable_session(
    state: &AppState,
    auth: &AuthContext,
    session_header: Option<&str>,
    request: JsonRpcRequest,
) -> Result<Response, ApiError> {
    let session_id = state.mcp.touch_streamable_session(session_header).await;
    let registry = state.mcp.streamable_sessions();
    let sse_rx = registry.open(session_id.clone()).await;

    let response = state.mcp.handle(request, auth).await;
    push_response_and_notifications(state, &session_id, &response).await?;

    let mut notify_rx = state.mcp.subscribe_notifications();
    let registry_bg = registry.clone();
    let session_bg = session_id.clone();
    tokio::spawn(async move {
        loop {
            match notify_rx.recv().await {
                Ok(notification) => {
                    if let Ok(data) = serde_json::to_string(&notification) {
                        if !registry_bg.push(&session_bg, data).await {
                            break;
                        }
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            }
        }
    });

    let registry_cleanup = registry.clone();
    let session_cleanup = session_id.clone();
    let stream = futures::stream::unfold(sse_rx, move |mut rx| {
        let registry_cleanup = registry_cleanup.clone();
        let session_cleanup = session_cleanup.clone();
        async move {
            match rx.recv().await {
                Some(data) => Some((Ok::<Event, Infallible>(Event::default().data(data)), rx)),
                None => {
                    registry_cleanup.close(&session_cleanup).await;
                    None
                }
            }
        }
    });

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

async fn push_response_and_notifications(
    state: &AppState,
    session_id: &str,
    response: &JsonRpcResponse,
) -> Result<(), ApiError> {
    let registry = state.mcp.streamable_sessions();
    let json = serde_json::to_string(response).map_err(|e| ApiError::Internal(e.to_string()))?;
    if !registry.push(session_id, json).await {
        return Err(ApiError::Internal("streamable session closed".into()));
    }
    for notification in state.mcp.take_pending_notifications().await {
        let data =
            serde_json::to_string(&notification).map_err(|e| ApiError::Internal(e.to_string()))?;
        if !registry.push(session_id, data).await {
            break;
        }
    }
    Ok(())
}

/// Close an open streamable session (`DELETE /mcp/streamable`, Cluster 60).
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
    state.mcp.streamable_sessions().close(session_id).await;
    Ok(StatusCode::NO_CONTENT)
}

/// Server→client SSE stream for a streamable session (`GET /mcp/streamable`,
/// Cluster 146). Delivers unsolicited server notifications (e.g. resource
/// updates) per the MCP spec's server-initiated GET stream; touches and echoes
/// an open `Mcp-Session-Id` when supplied.
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
    let session_id = match headers.get("mcp-session-id").and_then(|v| v.to_str().ok()) {
        Some(id) if !id.is_empty() && registry.is_open(id).await => {
            registry.touch(id).await;
            Some(id.to_string())
        }
        _ => None,
    };

    let rx = state.mcp.subscribe_notifications();
    let stream = BroadcastStream::new(rx).filter_map(|item| {
        let notification = item.ok()?;
        serde_json::to_string(&notification)
            .ok()
            .map(|data| Ok::<Event, Infallible>(Event::default().data(data)))
    });

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

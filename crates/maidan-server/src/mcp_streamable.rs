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
    body: axum::body::Bytes,
) -> Result<Response, ApiError> {
    if !auth.bypass {
        auth.require_capability(WORKSPACE_READ)
            .map_err(|_| ApiError::Forbidden("missing workspace:read capability".into()))?;
    }
    crate::mcp::validate_protocol_version(&headers)?;

    // Parse once as a value so a client's *response* (to a server→client
    // request) can be told apart from a request/notification.
    let value: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => return Ok(Json(JsonRpcResponse::parse_error()).into_response()),
    };
    let session_header = headers.get("mcp-session-id").and_then(|v| v.to_str().ok());

    // A JSON-RPC response (has `id`, no `method`) answers a server→client
    // request — route it to the awaiting caller rather than dispatching it.
    if value.get("method").is_none() && value.get("id").is_some() {
        if let Some(sid) = session_header.filter(|s| !s.is_empty()) {
            state.mcp.resolve_client_response(sid, value).await;
        }
        return Ok(StatusCode::ACCEPTED.into_response());
    }

    let request: JsonRpcRequest = match serde_json::from_value(value) {
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
    // Mux onto the open SSE leg (202). If that leg has since dropped — the
    // session survives it now, for reconnect — the response was still logged
    // for replay; answer it inline (200) rather than failing.
    let mut resp = if push_response_and_notifications(state, session_id, &response)
        .await
        .is_ok()
    {
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
    let session_id = state.mcp.touch_streamable_session(session_header).await;
    let registry = state.mcp.streamable_sessions();
    let sse_rx = registry.open(session_id.clone()).await;

    // Record the client's declared capabilities so the server can gate
    // server→client requests (sampling / roots / elicitation) on them.
    if request.method == "initialize" {
        let capabilities = request
            .params
            .get("capabilities")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        registry
            .set_client_capabilities(&session_id, capabilities)
            .await;
    }

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

    // Resumability: with an open session and a `Last-Event-ID`, replay the
    // retained frames after that id before the live stream (Cluster 147).
    let replay_frames = match (&session_id, last_event_id(&headers)) {
        (Some(id), Some(after)) => registry.replay_after(id, after).await,
        _ => Vec::new(),
    };
    let replay = futures::stream::iter(replay_frames.into_iter().map(|(event_id, data)| {
        Ok::<Event, Infallible>(Event::default().id(event_id.to_string()).data(data))
    }));

    let rx = state.mcp.subscribe_notifications();
    let live = BroadcastStream::new(rx).filter_map(|item| {
        let notification = item.ok()?;
        serde_json::to_string(&notification)
            .ok()
            .map(|data| Ok::<Event, Infallible>(Event::default().data(data)))
    });
    let stream = replay.chain(live);

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

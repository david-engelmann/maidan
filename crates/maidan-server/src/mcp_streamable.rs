//! MCP streamable HTTP: JSON-RPC response plus live notifications on one SSE stream
//! (`POST /mcp/streamable`). Cluster 27; session mux Cluster 35.

use std::convert::Infallible;
use std::time::Duration;

use axum::response::sse::{Event, KeepAlive};
use axum::{
    extract::State,
    http::{HeaderMap, HeaderName, HeaderValue},
    response::{IntoResponse, Response, Sse},
    Extension, Json,
};
use maidan_auth::{capability::WORKSPACE_READ, AuthContext};
use maidan_mcp::{JsonRpcRequest, JsonRpcResponse};

use crate::error::ApiError;
use crate::state::AppState;

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

    let request: JsonRpcRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(_) => return Ok(Json(JsonRpcResponse::parse_error()).into_response()),
    };

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
    let mut resp = Json(response).into_response();
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

//! MCP streamable HTTP: JSON-RPC response plus live notifications on one SSE stream
//! (`POST /mcp/streamable`). Cluster 27; `GET /mcp/notifications` remains for v16 clients.

use std::convert::Infallible;
use std::time::Duration;

use axum::response::sse::KeepAlive;
use axum::{
    extract::State,
    http::HeaderMap,
    response::{sse::Event, IntoResponse, Response, Sse},
    Extension, Json,
};
use maidan_auth::{capability::WORKSPACE_READ, AuthContext};
use maidan_mcp::{JsonRpcRequest, JsonRpcResponse};
use tokio_stream::wrappers::UnboundedReceiverStream;

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
    let session_id = state.mcp.touch_streamable_session(session_header).await;

    let mut notify_rx = state.mcp.subscribe_notifications();
    let response = state.mcp.handle(request, &auth).await;
    let pending = state.mcp.take_pending_notifications().await;

    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    tx.send(sse_data(
        serde_json::to_string(&response).map_err(|e| ApiError::Internal(e.to_string()))?,
    ))
    .map_err(|_| ApiError::Internal("stream closed".into()))?;
    for notification in pending {
        tx.send(sse_data(
            serde_json::to_string(&notification).map_err(|e| ApiError::Internal(e.to_string()))?,
        ))
        .map_err(|_| ApiError::Internal("stream closed".into()))?;
    }

    tokio::spawn(async move {
        loop {
            match notify_rx.recv().await {
                Ok(notification) => {
                    if let Ok(data) = serde_json::to_string(&notification) {
                        if tx.send(sse_data(data)).is_err() {
                            break;
                        }
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            }
        }
    });

    let stream = UnboundedReceiverStream::new(rx);
    let mut resp = Sse::new(stream)
        .keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(15))
                .text("ping"),
        )
        .into_response();
    if let Ok(value) = axum::http::HeaderValue::from_str(&session_id) {
        resp.headers_mut()
            .insert(axum::http::HeaderName::from_static("mcp-session-id"), value);
    }
    Ok(resp)
}

fn sse_data(data: String) -> Result<Event, Infallible> {
    Ok(Event::default().data(data))
}

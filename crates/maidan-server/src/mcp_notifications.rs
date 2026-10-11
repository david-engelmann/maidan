//! SSE stream for MCP resource subscription notifications (`GET /mcp/notifications`).
//!
//! Distinct from [`crate::mcp_stream`] (`GET /mcp/stream`), which replays workspace bus
//! events. This endpoint carries JSON-RPC notifications such as
//! `notifications/resources/updated` for HTTP MCP clients: only for what this
//! caller subscribed to over stateless `POST /mcp`, for as long as it can read
//! it. An open listener keeps those subscriptions alive.

use std::convert::Infallible;
use std::time::Duration;

use axum::{
    extract::State,
    response::sse::{Event, KeepAlive, Sse},
    Extension,
};
use maidan_auth::{capability::WORKSPACE_READ, AuthContext};
use maidan_mcp::McpSession;
use tokio_stream::{Stream, StreamExt as _};

use crate::error::ApiError;
use crate::state::AppState;

pub async fn stream(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
) -> Result<Sse<impl Stream<Item = Result<Event, Infallible>>>, ApiError> {
    if !auth.bypass {
        auth.require_capability(WORKSPACE_READ)
            .map_err(|_| ApiError::Forbidden("missing workspace:read capability".into()))?;
    }

    let notification_stream = state
        .mcp
        .listen(&auth, McpSession::Stateless)
        .await
        .into_stream()
        .filter_map(|notification| {
            serde_json::to_string(&notification)
                .ok()
                .map(|data| Ok(Event::default().data(data)))
        });

    let notification_stream =
        crate::stream_guard::guard(state.store.clone(), &auth, notification_stream);

    Ok(Sse::new(notification_stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("ping"),
    ))
}

#[cfg(test)]
mod tests {
    use maidan_mcp::JsonRpcNotification;
    use serde_json::json;

    #[test]
    fn notification_serializes_as_json_rpc() {
        let n = JsonRpcNotification::new(
            "notifications/resources/updated",
            json!({ "uri": "maidan://threads/x" }),
        );
        let line = serde_json::to_string(&n).unwrap();
        assert!(line.contains("notifications/resources/updated"));
    }
}

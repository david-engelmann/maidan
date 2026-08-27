//! HTTP transport for the MCP server: `POST /mcp` accepts a single JSON-RPC 2.0
//! request **or a batch** (a top-level array) and returns the corresponding
//! response(s). Notifications (requests without an `id`) are executed for effect
//! and answered with `202 Accepted` and no body. The `MCP-Protocol-Version`
//! header is validated against the supported set.
//! Resource subscription notifications also stream on `GET /mcp/notifications`.

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Extension, Json,
};
use maidan_auth::AuthContext;
use maidan_mcp::{is_supported_protocol_version, JsonRpcError, JsonRpcRequest, JsonRpcResponse};

use crate::error::ApiError;
use crate::state::AppState;

/// Validate the `MCP-Protocol-Version` header (MCP spec: clients send it on
/// requests after `initialize`). Absent is allowed for backwards compatibility;
/// present-but-unsupported is a `400`.
pub(crate) fn validate_protocol_version(headers: &HeaderMap) -> Result<(), ApiError> {
    if let Some(value) = headers.get("mcp-protocol-version") {
        let version = value
            .to_str()
            .map_err(|_| ApiError::BadRequest("invalid MCP-Protocol-Version header".into()))?;
        if !is_supported_protocol_version(version) {
            return Err(ApiError::BadRequest(format!(
                "unsupported MCP-Protocol-Version: {version}"
            )));
        }
    }
    Ok(())
}

/// The MCP revision (`2026-07-28`) whose transport is stateless: no protocol-level
/// sessions, `Mcp-Session-Id` gone, any request lands cold. The streamable POST
/// uses this to serve such a request inline without minting a session.
pub(crate) const STATELESS_PROTOCOL_VERSION: &str = "2026-07-28";

/// Whether the request declares the stateless `2026-07-28` revision via the
/// `MCP-Protocol-Version` header (already validated by [`validate_protocol_version`]).
pub(crate) fn is_stateless_request(headers: &HeaderMap) -> bool {
    headers
        .get("mcp-protocol-version")
        .and_then(|v| v.to_str().ok())
        == Some(STATELESS_PROTOCOL_VERSION)
}

pub async fn handler(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if let Err(err) = validate_protocol_version(&headers) {
        return err.into_response();
    }
    let value: serde_json::Value = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(_) => return Json(JsonRpcResponse::parse_error()).into_response(),
    };
    match value {
        serde_json::Value::Array(items) => batch_response(&state, &auth, items).await,
        other => single_response(&state, &auth, other).await,
    }
}

async fn single_response(
    state: &AppState,
    auth: &AuthContext,
    value: serde_json::Value,
) -> Response {
    let request: JsonRpcRequest = match serde_json::from_value(value) {
        Ok(r) => r,
        Err(_) => return Json(JsonRpcResponse::parse_error()).into_response(),
    };
    // A notification (no `id`) is executed for effect and returns no body.
    if request.id.is_none() {
        let _ = state.mcp.handle(request, auth).await;
        return StatusCode::ACCEPTED.into_response();
    }
    if let Err(resp) = crate::mcp_quota::enforce_mcp_quota(state, auth, &request).await {
        return Json(resp).into_response();
    }
    Json(state.mcp.handle(request, auth).await).into_response()
}

async fn batch_response(
    state: &AppState,
    auth: &AuthContext,
    items: Vec<serde_json::Value>,
) -> Response {
    // JSON-RPC 2.0: an empty batch array is itself an invalid request.
    if items.is_empty() {
        return Json(JsonRpcResponse::failure(
            serde_json::Value::Null,
            JsonRpcError {
                code: -32600,
                message: "invalid request: empty batch".into(),
                data: None,
            },
        ))
        .into_response();
    }
    let mut responses = Vec::new();
    for item in items {
        let request: JsonRpcRequest = match serde_json::from_value(item) {
            Ok(r) => r,
            Err(_) => {
                responses.push(JsonRpcResponse::parse_error());
                continue;
            }
        };
        let is_notification = request.id.is_none();
        if let Err(resp) = crate::mcp_quota::enforce_mcp_quota(state, auth, &request).await {
            if !is_notification {
                responses.push(resp);
            }
            continue;
        }
        let resp = state.mcp.handle(request, auth).await;
        if !is_notification {
            responses.push(resp);
        }
    }
    // A batch of only notifications yields no response body.
    if responses.is_empty() {
        StatusCode::ACCEPTED.into_response()
    } else {
        Json(responses).into_response()
    }
}

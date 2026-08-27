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

/// The name a routing gateway would put in `Mcp-Name` for a request: the tool /
/// prompt name, or the resource uri. `None` for methods that name no target
/// (`tools/list`, `initialize`, …).
fn request_routing_name(request: &JsonRpcRequest) -> Option<&str> {
    match request.method.as_str() {
        "tools/call" | "prompts/get" => request.params.get("name").and_then(|v| v.as_str()),
        "resources/read" | "resources/subscribe" | "resources/unsubscribe" => {
            request.params.get("uri").and_then(|v| v.as_str())
        }
        _ => None,
    }
}

/// Validate the SEP-2243 routing headers (`Mcp-Method` / `Mcp-Name`) against the
/// request body. Both are optional (a gateway adds them so it can route/authorize
/// without parsing JSON; a direct client may omit them). When present they MUST
/// match the body — a gateway that routed on a header must not be handed a
/// contradicting body — so a mismatch is a `400`. An `Mcp-Name` on a method that
/// names no target is ignored (the body does no more than the header authorized).
pub(crate) fn validate_routing_headers(
    headers: &HeaderMap,
    request: &JsonRpcRequest,
) -> Result<(), ApiError> {
    if let Some(m) = headers.get("mcp-method").and_then(|v| v.to_str().ok()) {
        if m != request.method {
            return Err(ApiError::BadRequest(format!(
                "Mcp-Method header {:?} does not match request method {:?}",
                m, request.method
            )));
        }
    }
    if let Some(n) = headers.get("mcp-name").and_then(|v| v.to_str().ok()) {
        if let Some(name) = request_routing_name(request) {
            if name != n {
                return Err(ApiError::BadRequest(format!(
                    "Mcp-Name header {n:?} does not match request target {name:?}"
                )));
            }
        }
    }
    Ok(())
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
        // Routing headers describe a single op; a batch names many, so they are
        // validated per single request, not against an array.
        serde_json::Value::Array(items) => batch_response(&state, &auth, items).await,
        other => single_response(&state, &auth, &headers, other).await,
    }
}

async fn single_response(
    state: &AppState,
    auth: &AuthContext,
    headers: &HeaderMap,
    value: serde_json::Value,
) -> Response {
    let request: JsonRpcRequest = match serde_json::from_value(value) {
        Ok(r) => r,
        Err(_) => return Json(JsonRpcResponse::parse_error()).into_response(),
    };
    if let Err(err) = validate_routing_headers(headers, &request) {
        return err.into_response();
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;
    use serde_json::json;

    fn req(method: &str, params: serde_json::Value) -> JsonRpcRequest {
        JsonRpcRequest {
            jsonrpc: "2.0".into(),
            id: Some(json!(1)),
            method: method.into(),
            params,
        }
    }

    #[test]
    fn routing_headers_absent_are_ok() {
        let r = req("tools/call", json!({ "name": "search" }));
        assert!(validate_routing_headers(&HeaderMap::new(), &r).is_ok());
    }

    #[test]
    fn matching_routing_headers_are_ok() {
        let mut h = HeaderMap::new();
        h.insert("mcp-method", HeaderValue::from_static("tools/call"));
        h.insert("mcp-name", HeaderValue::from_static("search"));
        let r = req("tools/call", json!({ "name": "search" }));
        assert!(validate_routing_headers(&h, &r).is_ok());
    }

    #[test]
    fn mcp_method_mismatch_is_rejected() {
        let mut h = HeaderMap::new();
        h.insert("mcp-method", HeaderValue::from_static("tools/list"));
        let r = req("tools/call", json!({ "name": "search" }));
        assert!(validate_routing_headers(&h, &r).is_err());
    }

    #[test]
    fn mcp_name_mismatch_is_rejected() {
        let mut h = HeaderMap::new();
        h.insert("mcp-name", HeaderValue::from_static("evil"));
        let r = req("tools/call", json!({ "name": "search" }));
        assert!(validate_routing_headers(&h, &r).is_err());
    }

    #[test]
    fn mcp_name_on_a_method_that_names_no_target_is_ignored() {
        // A superfluous Mcp-Name on tools/list is safe (the body does no more than
        // the header authorized), so it's not a mismatch.
        let mut h = HeaderMap::new();
        h.insert("mcp-name", HeaderValue::from_static("whatever"));
        let r = req("tools/list", json!({}));
        assert!(validate_routing_headers(&h, &r).is_ok());
    }

    #[test]
    fn mcp_name_matches_resource_uri() {
        let mut h = HeaderMap::new();
        h.insert("mcp-name", HeaderValue::from_static("threads/42"));
        let r = req("resources/read", json!({ "uri": "threads/42" }));
        assert!(validate_routing_headers(&h, &r).is_ok());
    }
}

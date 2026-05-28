//! HTTP transport for the MCP server: `POST /mcp` accepts a single
//! JSON-RPC 2.0 request and returns the corresponding response.
//! Resource subscription notifications also stream on `GET /mcp/notifications`.

use axum::{extract::State, response::IntoResponse, Extension, Json};
use maidan_auth::AuthContext;
use maidan_mcp::{JsonRpcRequest, JsonRpcResponse};

use crate::state::AppState;

pub async fn handler(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    body: axum::body::Bytes,
) -> impl IntoResponse {
    let request: JsonRpcRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(_) => return Json(JsonRpcResponse::parse_error()).into_response(),
    };
    Json(state.mcp.handle(request, &auth).await).into_response()
}

//! HTTP transport for the MCP server: `POST /mcp` accepts a single
//! JSON-RPC 2.0 request and returns the corresponding response.
//! Streamable HTTP (SSE notifications) arrives in a later cluster.

use axum::{extract::State, response::IntoResponse, Json};
use maidan_mcp::{JsonRpcRequest, JsonRpcResponse, McpServer};

use crate::state::AppState;

pub async fn handler(State(state): State<AppState>, body: axum::body::Bytes) -> impl IntoResponse {
    let request: JsonRpcRequest = match serde_json::from_slice(&body) {
        Ok(r) => r,
        Err(_) => return Json(JsonRpcResponse::parse_error()).into_response(),
    };
    let server = McpServer::new(state.store.clone());
    Json(server.handle(request).await).into_response()
}

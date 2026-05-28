//! HTTP transport for the MCP server: `POST /mcp` accepts a single
//! JSON-RPC 2.0 request and returns the corresponding response.
//! `resources/subscribe` notifications currently ship on stdio only.

use axum::{extract::State, response::IntoResponse, Extension, Json};
use maidan_auth::AuthContext;
use maidan_mcp::{JsonRpcRequest, JsonRpcResponse, McpServer};

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
    let server = McpServer::new(
        state.store.clone(),
        state.artifacts.clone(),
        state.search.clone(),
        state.embedding_provider.clone(),
    );
    Json(server.handle(request, &auth).await).into_response()
}

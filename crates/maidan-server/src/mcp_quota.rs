//! MCP JSON-RPC quota hooks (Cluster 64).

use maidan_auth::AuthContext;
use maidan_mcp::{JsonRpcError, JsonRpcRequest, JsonRpcResponse};

use crate::error::ApiError;
use crate::quota;
use crate::state::AppState;

pub fn tool_capability(request: &JsonRpcRequest) -> Option<&'static str> {
    if request.method != "tools/call" {
        return None;
    }
    let name = request.params.get("name")?.as_str()?;
    maidan_mcp::tools::required_capability(name).ok()
}

pub async fn enforce_mcp_quota(
    state: &AppState,
    auth: &AuthContext,
    request: &JsonRpcRequest,
) -> Result<(), JsonRpcResponse> {
    if auth.bypass {
        return Ok(());
    }
    let Some(token_id) = auth.token_id else {
        return Ok(());
    };
    let Some(cap) = tool_capability(request) else {
        return Ok(());
    };
    if let Err(err) = quota::enforce_token_quota(state, token_id, cap).await {
        let id = request.id.clone().unwrap_or(serde_json::Value::Null);
        let msg = match err {
            ApiError::TooManyRequests(m) => m,
            ApiError::Forbidden(m) => m,
            ApiError::BadRequest(m) => m,
            ApiError::Unauthorized => "unauthorized".into(),
            ApiError::NotFound => "not found".into(),
            ApiError::Conflict(m) => m,
            ApiError::Internal(m) => m,
        };
        return Err(JsonRpcResponse::failure(
            id,
            JsonRpcError {
                code: -32029,
                message: msg,
                data: None,
            },
        ));
    }
    Ok(())
}

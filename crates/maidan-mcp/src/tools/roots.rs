//! `roots/list` tool (Cluster 193).
//!
//! Asks the client which roots (filesystem / workspace boundaries) it exposes,
//! via the MCP `roots/list` server→client request — the third `request_client`
//! verb, after sampling (`summarize_thread`, Cluster 155) and elicitation
//! (`request_approval`, Cluster 174). The transport (Cluster 148, GET-stream
//! delivery from 154) and capability gating already existed; this is its first
//! organic caller.

use serde_json::{json, Value};

use super::content_json;
use crate::error::McpError;

/// List the roots the connected client exposes. Requires a streamable session
/// whose client declared the `roots` capability (checked by `request_client`);
/// returns the client's `{roots: [...]}` verbatim.
pub(super) async fn list_roots(
    server: &crate::server::McpServer,
    session_id: Option<&str>,
    _args: &Value,
) -> Result<Value, McpError> {
    let session = session_id.ok_or_else(|| {
        McpError::InvalidParams(
            "list_roots requires a streamable session (open GET /mcp/streamable) whose client \
             supports roots"
                .into(),
        )
    })?;
    let result = server
        .request_client(session, "roots/list", json!({}))
        .await?;
    Ok(content_json(&result))
}

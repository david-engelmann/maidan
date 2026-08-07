//! Human-in-the-loop approval tool (Cluster 174).
//!
//! `request_approval` asks the *human* on the other end of a streamable MCP
//! session to approve or reject an action, via the spec's `elicitation/create`
//! server→client request (Cluster 148 transport, GET-stream delivery from 154).
//! It's the elicitation analogue of the sampling-backed `summarize_thread`
//! (Cluster 155): a gate an agent can `await` before doing something sensitive.

use serde::Deserialize;
use serde_json::{json, Value};

use super::content_json;
use crate::error::McpError;

#[derive(Deserialize)]
struct RequestApprovalArgs {
    /// Human-readable description of what needs approval.
    prompt: String,
    /// Optional JSON Schema for structured detail the human may supply
    /// alongside their decision (MCP `requestedSchema`). Defaults to an empty
    /// object — the accept/decline action alone carries the decision.
    #[serde(default)]
    schema: Option<Value>,
}

/// Elicit a human approve/reject decision over the session's client.
///
/// Requires a streamable session whose client declared the `elicitation`
/// capability. Returns `{approved, action, content}` where `approved` is true
/// iff the human chose `accept`; `decline`/`cancel` (or a timeout, surfaced as
/// an error) mean not approved.
pub(super) async fn request_approval(
    server: &crate::server::McpServer,
    session_id: Option<&str>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: RequestApprovalArgs = serde_json::from_value(args.clone())?;
    let session = session_id.ok_or_else(|| {
        McpError::InvalidParams(
            "request_approval requires a streamable session (open GET /mcp/streamable) whose \
             client supports elicitation"
                .into(),
        )
    })?;

    // MCP `elicitation/create`: the client presents `message` to the human and
    // collects a response conforming to `requestedSchema`, returning
    // `{action: accept|decline|cancel, content?}`.
    let requested_schema = a.schema.unwrap_or_else(
        || json!({ "type": "object", "properties": {}, "additionalProperties": false }),
    );
    let params = json!({
        "message": a.prompt,
        "requestedSchema": requested_schema,
    });
    let result = server
        .request_client(session, "elicitation/create", params)
        .await?;

    let action = result.get("action").and_then(|v| v.as_str()).unwrap_or("");
    let approved = action == "accept";
    Ok(content_json(&json!({
        "approved": approved,
        "action": action,
        "content": result.get("content").cloned().unwrap_or(Value::Null),
    })))
}

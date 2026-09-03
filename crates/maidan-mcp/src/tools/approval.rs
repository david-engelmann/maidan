//! Human-in-the-loop approval tools (Cluster 174, reworked in Cluster 350).
//!
//! `request_approval` opens a durable [`ApprovalGate`](maidan_types::ApprovalGate)
//! and returns an `input_required` result **without blocking** — the agent is
//! free to do other work and poll `get_approval_gate` (or await the
//! `ThreadResultSet`-style signal in a later cluster) for the human's decision.
//! This replaces the pre-350 model where the tool call parked up to 30s on an
//! in-memory oneshot: the gate is now persisted, survives a dropped connection,
//! and is answerable by a human over the `/ui` (Cluster 350.3+). "Silence is not
//! consent" — a gate is resolved only by an explicit accept/decline/cancel.

use serde::Deserialize;
use serde_json::{json, Value};

use super::content_json;
use crate::error::McpError;
use maidan_auth::AuthContext;
use maidan_types::{ApprovalGateId, NewApprovalGate};

#[derive(Deserialize)]
struct RequestApprovalArgs {
    /// Human-readable description of what needs approval.
    prompt: String,
    /// Optional JSON Schema for structured detail the human may supply alongside
    /// their decision (MCP `requestedSchema`). Persisted on the gate.
    #[serde(default)]
    schema: Option<Value>,
    /// Optional thread to attach the gate to (Cluster 350.6, N6). While the gate
    /// is `pending`, `claim_next` will not hand the thread to an agent — the
    /// required human is a *claim gate*, not a notification preference.
    #[serde(default)]
    thread_id: Option<uuid::Uuid>,
}

/// Open a durable human-approval gate and return `input_required`.
///
/// The gate is created `pending` and attributed to the caller
/// (`auth.member_id`); a human resolves it later to accept/decline/cancel. The
/// tool does **not** block — poll `get_approval_gate` with the returned
/// `gate_id` for the outcome. Requires `workspace:write` (it persists a gate).
pub(super) async fn request_approval(
    server: &crate::server::McpServer,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: RequestApprovalArgs = serde_json::from_value(args.clone())?;
    let gate = server
        .store
        .create_approval_gate(&NewApprovalGate {
            workspace_id: auth.workspace_id,
            thread_id: a.thread_id.map(maidan_types::ThreadId),
            requested_by: auth.member_id,
            prompt: a.prompt,
            schema: a.schema,
        })
        .await?;
    Ok(content_json(&json!({
        "status": "input_required",
        "gate_id": gate.id,
    })))
}

#[derive(Deserialize)]
struct GetApprovalGateArgs {
    gate_id: ApprovalGateId,
}

/// Poll a durable approval gate by id (Cluster 350). Returns the gate — its
/// `state` is `pending` until a human answers, then `accepted`/`declined`/
/// `cancelled` with any `content` they supplied. `null` if no such gate exists
/// in the caller's workspace. Requires `workspace:read`.
pub(super) async fn get_approval_gate(
    server: &crate::server::McpServer,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: GetApprovalGateArgs = serde_json::from_value(args.clone())?;
    let gate = server.store.get_approval_gate(a.gate_id).await?;
    match gate {
        // Scope to the caller's workspace (bypass sees all); an out-of-workspace
        // id reads as "not found" — no cross-tenant existence oracle.
        Some(g) if auth.bypass || g.workspace_id == auth.workspace_id => {
            Ok(content_json(&serde_json::to_value(g)?))
        }
        _ => Ok(content_json(&Value::Null)),
    }
}

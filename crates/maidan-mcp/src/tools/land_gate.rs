//! Land-gate pointer MCP tools, the twins of the REST surface. Writes =
//! `thread:transition`; reads = `workspace:read`. Thread access is the
//! pre-dispatch `thread_id` gate.

use std::sync::Arc;

use maidan_auth::AuthContext;
use maidan_store::Store;
use maidan_types::{LandColor, LandGateStatus, ThreadId};
use serde::Deserialize;
use serde_json::Value;

use super::content_json;
use crate::error::McpError;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SetArgs {
    thread_id: uuid::Uuid,
    status: LandGateStatus,
    #[serde(default)]
    artifact_sha: Option<String>,
    #[serde(default)]
    land: Option<LandColor>,
}

/// Record a LandGate pointer as the caller. The caller must have declared the
/// `land_gate` skill; amber is not a land.
pub(super) async fn set_land_gate(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: SetArgs = crate::tools::parse_args(args)?;
    let sha = a
        .artifact_sha
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let standing = store
        .set_land_gate_pointer(ThreadId(a.thread_id), auth.member_id, a.status, sha, a.land)
        .await?;
    Ok(content_json(&standing))
}

#[derive(Deserialize)]
struct ThreadArg {
    thread_id: uuid::Uuid,
}

/// Read a thread's LandGate standing.
pub(super) async fn get_land_gate(store: &Arc<dyn Store>, args: &Value) -> Result<Value, McpError> {
    let a: ThreadArg = crate::tools::parse_args(args)?;
    let standing = store.get_land_gate_standing(ThreadId(a.thread_id)).await?;
    Ok(content_json(&standing))
}

/// Every land-gate verdict recorded on the thread, oldest first.
pub(super) async fn list_land_gate_history(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ThreadArg = crate::tools::parse_args(args)?;
    let history = store.list_land_gate_history(ThreadId(a.thread_id)).await?;
    Ok(content_json(&history))
}

/// Arm the LandGate close-gate without a pointer yet.
pub(super) async fn require_land_gate(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ThreadArg = crate::tools::parse_args(args)?;
    let standing = store.require_land_gate(ThreadId(a.thread_id)).await?;
    Ok(content_json(&standing))
}

/// Clear the LandGate pointer / requirement.
pub(super) async fn clear_land_gate(
    store: &Arc<dyn Store>,
    auth: &maidan_auth::AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ThreadArg = crate::tools::parse_args(args)?;
    let workspace_id = super::thread_workspace(store.as_ref(), ThreadId(a.thread_id)).await?;
    let cleared = store
        .clear_land_gate_audited(
            ThreadId(a.thread_id),
            maidan_types::NewAuditEvent {
                scope: maidan_types::AuditScope::Workspace(workspace_id),
                actor_id: Some(auth.actor_id),
                action: "land_gate.clear".into(),
                target_kind: Some("thread".into()),
                target_id: Some(a.thread_id),
                metadata: serde_json::json!({ "surface": "mcp" }),
            },
        )
        .await?;
    Ok(content_json(&serde_json::json!({ "cleared": cleared })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AdviseArgs {
    thread_id: uuid::Uuid,
    state: Value,
    #[serde(default)]
    instructions: Option<String>,
    #[serde(default)]
    thresholds: Option<Value>,
}

/// Ask the optional advisor for a land-gate recommendation. Twin of
/// `POST /threads/{id}/land-gate/advice`. Read-only: it does not write the
/// pointer or the requirement. `NotFound` when no advisor is configured.
pub(super) async fn advise_land_gate(
    server: &crate::server::McpServer,
    args: &Value,
) -> Result<Value, McpError> {
    let a: AdviseArgs = crate::tools::parse_args(args)?;
    let thread_id = a.thread_id;
    let Some(advisor) = server.land_gate_advisor() else {
        return Err(McpError::NotFound);
    };
    let mut request = serde_json::json!({ "state": a.state });
    if let Some(instructions) = a.instructions {
        request["instructions"] = Value::String(instructions);
    }
    if let Some(thresholds) = a.thresholds {
        request["thresholds"] = thresholds;
    }
    match advisor.advise(request).await {
        Ok(advice) => Ok(content_json(&advice)),
        Err(crate::land_gate_advice::LandGateAdviseError::Invalid(error)) => {
            Err(McpError::InvalidParams(error))
        }
        Err(crate::land_gate_advice::LandGateAdviseError::Unavailable(error)) => {
            tracing::warn!(%error, %thread_id, "land-gate advisor request failed");
            Err(McpError::Internal(
                "land-gate advisor unavailable; the authoritative gate is unchanged".into(),
            ))
        }
    }
}

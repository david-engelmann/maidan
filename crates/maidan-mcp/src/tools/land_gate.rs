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
    let a: SetArgs = serde_json::from_value(args.clone())?;
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
    let a: ThreadArg = serde_json::from_value(args.clone())?;
    let standing = store.get_land_gate_standing(ThreadId(a.thread_id)).await?;
    Ok(content_json(&standing))
}

/// Arm the LandGate close-gate without a pointer yet.
pub(super) async fn require_land_gate(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ThreadArg = serde_json::from_value(args.clone())?;
    let standing = store.require_land_gate(ThreadId(a.thread_id)).await?;
    Ok(content_json(&standing))
}

/// Clear the LandGate pointer / requirement.
pub(super) async fn clear_land_gate(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ThreadArg = serde_json::from_value(args.clone())?;
    let cleared = store.clear_land_gate(ThreadId(a.thread_id)).await?;
    Ok(content_json(&serde_json::json!({ "cleared": cleared })))
}

//! Soundcheck gate pointer MCP tools (Cluster 385.3, Wave 2 #25 remainder).
//! The REST twin is Cluster 385.3. Writes = `thread:transition`; reads =
//! `workspace:read`. Thread access is the pre-dispatch `thread_id` gate.

use std::sync::Arc;

use maidan_auth::AuthContext;
use maidan_store::Store;
use maidan_types::{LandColor, SoundcheckStatus, ThreadId};
use serde::Deserialize;
use serde_json::Value;

use super::content_json;
use crate::error::McpError;

#[derive(Deserialize)]
struct SetArgs {
    thread_id: uuid::Uuid,
    status: SoundcheckStatus,
    #[serde(default)]
    artifact_sha: Option<String>,
    #[serde(default)]
    land: Option<LandColor>,
}

/// Record a Soundcheck pointer as the caller (Cluster 385.3). The caller
/// must have declared the `soundcheck` skill; amber is not a land.
pub(super) async fn set_soundcheck(
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
        .set_soundcheck_pointer(ThreadId(a.thread_id), auth.member_id, a.status, sha, a.land)
        .await?;
    Ok(content_json(&standing))
}

#[derive(Deserialize)]
struct ThreadArg {
    thread_id: uuid::Uuid,
}

/// Read a thread's Soundcheck standing (Cluster 385.3).
pub(super) async fn get_soundcheck(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ThreadArg = serde_json::from_value(args.clone())?;
    let standing = store.get_soundcheck_standing(ThreadId(a.thread_id)).await?;
    Ok(content_json(&standing))
}

/// Arm the Soundcheck close-gate without a pointer yet (Cluster 385.3).
pub(super) async fn require_soundcheck(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ThreadArg = serde_json::from_value(args.clone())?;
    let standing = store.require_soundcheck(ThreadId(a.thread_id)).await?;
    Ok(content_json(&standing))
}

/// Clear the Soundcheck pointer / requirement (Cluster 385.3).
pub(super) async fn clear_soundcheck(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ThreadArg = serde_json::from_value(args.clone())?;
    let cleared = store.clear_soundcheck(ThreadId(a.thread_id)).await?;
    Ok(content_json(&serde_json::json!({ "cleared": cleared })))
}

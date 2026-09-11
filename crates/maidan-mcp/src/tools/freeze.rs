//! Member-freeze kill-switch MCP tools (Cluster 372.4, Wave 2 #20). An
//! orchestrator with `token:admin` can freeze a misbehaving member (dropping
//! their leases; `claim_next` then refuses them), unfreeze, and list the frozen.
//! The REST twin is Cluster 372.3.

use std::sync::Arc;

use maidan_auth::AuthContext;
use maidan_store::Store;
use maidan_types::MemberId;
use serde::Deserialize;
use serde_json::{json, Value};

use super::content_json;
use crate::error::McpError;

/// Verify the target member is in the caller's workspace (bypass exempt).
async fn ensure_same_workspace(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    member_id: MemberId,
) -> Result<(), McpError> {
    if auth.bypass {
        return Ok(());
    }
    let member = store.get_member(member_id).await?;
    if member.workspace_id != auth.workspace_id {
        return Err(McpError::InvalidParams(
            "member is not in the caller's workspace".into(),
        ));
    }
    Ok(())
}

#[derive(Deserialize)]
struct FreezeArgs {
    member_id: uuid::Uuid,
    #[serde(default)]
    reason: Option<String>,
}

/// Freeze a member (Cluster 372.4): drops their active leases and makes
/// `claim_next` refuse them. Returns the freeze + the count of claims released.
pub(super) async fn freeze_member(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: FreezeArgs = serde_json::from_value(args.clone())?;
    let member_id = MemberId(a.member_id);
    ensure_same_workspace(store, auth, member_id).await?;
    let reason = a.reason.as_deref().map(str::trim).filter(|r| !r.is_empty());
    let (freeze, released) = store
        .freeze_member(member_id, auth.member_id, reason)
        .await?;
    Ok(content_json(
        &json!({ "freeze": freeze, "released": released }),
    ))
}

#[derive(Deserialize)]
struct UnfreezeArgs {
    member_id: uuid::Uuid,
}

pub(super) async fn unfreeze_member(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: UnfreezeArgs = serde_json::from_value(args.clone())?;
    let member_id = MemberId(a.member_id);
    ensure_same_workspace(store, auth, member_id).await?;
    let unfrozen = store.unfreeze_member(member_id).await?;
    Ok(content_json(&json!({ "unfrozen": unfrozen })))
}

/// List the frozen members in the caller's workspace (Cluster 372.4).
pub(super) async fn list_frozen_members(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    _args: &Value,
) -> Result<Value, McpError> {
    let frozen = store.list_frozen_members(auth.workspace_id).await?;
    Ok(content_json(&frozen))
}

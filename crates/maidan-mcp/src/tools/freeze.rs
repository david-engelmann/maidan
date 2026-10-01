//! Member-freeze kill-switch MCP tools. An orchestrator with `token:admin` can
//! freeze a misbehaving member (dropping their leases; `claim_next` then
//! refuses them), unfreeze, and list the frozen. A freeze and an unfreeze that
//! lifts one publish `MemberFrozen` / `MemberUnfrozen`.

use std::sync::Arc;

use maidan_auth::AuthContext;
use maidan_store::Store;
use maidan_types::{AuditScope, MemberId, WorkspaceId};
use serde::Deserialize;
use serde_json::{json, Value};

use super::content_json;
use crate::error::McpError;

/// Verify the target member is in the caller's workspace (bypass exempt), and
/// return the member's workspace.
async fn ensure_same_workspace(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    member_id: MemberId,
) -> Result<WorkspaceId, McpError> {
    Ok(super::requested_member(store.as_ref(), auth, member_id)
        .await?
        .workspace_id)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FreezeArgs {
    member_id: uuid::Uuid,
    #[serde(default)]
    reason: Option<String>,
}

/// Freeze a member: drops their active leases and makes `claim_next` refuse
/// them. Returns the freeze + the count of claims released.
pub(super) async fn freeze_member(
    server: &crate::server::McpServer,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let store = &server.store;
    let a: FreezeArgs = serde_json::from_value(args.clone())?;
    let member_id = MemberId(a.member_id);
    let workspace_id = ensure_same_workspace(store, auth, member_id).await?;
    let reason = a.reason.as_deref().map(str::trim).filter(|r| !r.is_empty());
    let actor = auth.actor_id;
    let (freeze, released, stored) = store
        .freeze_member_audited(
            member_id,
            auth.member_id,
            reason,
            Box::new(move |(freeze, released)| maidan_types::NewAuditEvent {
                scope: AuditScope::Workspace(workspace_id),
                actor_id: Some(actor),
                action: "member.freeze".into(),
                target_kind: Some("member".into()),
                target_id: Some(member_id.0),
                metadata: json!({ "reason": freeze.reason, "released": released, "surface": "mcp" }),
            }),
        )
        .await?;
    server.publish_stored(&stored).await;
    Ok(content_json(
        &json!({ "freeze": freeze, "released": released }),
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct UnfreezeArgs {
    member_id: uuid::Uuid,
}

pub(super) async fn unfreeze_member(
    server: &crate::server::McpServer,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let store = &server.store;
    let a: UnfreezeArgs = serde_json::from_value(args.clone())?;
    let member_id = MemberId(a.member_id);
    let workspace_id = ensure_same_workspace(store, auth, member_id).await?;
    let stored = store
        .unfreeze_member_audited(
            member_id,
            auth.member_id,
            maidan_types::NewAuditEvent {
                scope: AuditScope::Workspace(workspace_id),
                actor_id: Some(auth.actor_id),
                action: "member.unfreeze".into(),
                target_kind: Some("member".into()),
                target_id: Some(member_id.0),
                metadata: json!({ "surface": "mcp" }),
            },
        )
        .await?;
    if let Some(stored) = &stored {
        server.publish_stored(stored).await;
    }
    Ok(content_json(&json!({ "unfrozen": stored.is_some() })))
}

/// List the frozen members in the caller's workspace.
pub(super) async fn list_frozen_members(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    _args: &Value,
) -> Result<Value, McpError> {
    let frozen = store.list_frozen_members(auth.workspace_id).await?;
    Ok(content_json(&frozen))
}

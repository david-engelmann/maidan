//! Spawn-budget configuration tools (Cluster 376.4, Wave 2 #23, G6/G-dev-3/W3)
//! — the MCP twin of the REST `/workspaces/:id/spawn-budget` surface. The budget
//! caps how far an agent family may fan out (children per parent, nesting depth,
//! tool calls per thread); the gate itself is Cluster 376.2 (thread create) and
//! 376.3 (message post). Applies to the caller's own workspace, like
//! `set_wip_limit`.

use std::sync::Arc;

use maidan_auth::AuthContext;
use maidan_store::Store;
use serde::Deserialize;
use serde_json::{json, Value};

use super::content_json;
use crate::error::McpError;

#[derive(Deserialize)]
struct SetSpawnBudgetArgs {
    #[serde(default)]
    max_children: Option<i64>,
    #[serde(default)]
    max_depth: Option<i64>,
    #[serde(default)]
    max_tools: Option<i64>,
}

/// Set the caller's workspace spawn budget (Cluster 376.4). A full replace — an
/// omitted or null axis is unlimited, so no arguments at all clears the budget;
/// `0` freezes an axis. `workspace:write`.
pub(super) async fn set_spawn_budget(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: SetSpawnBudgetArgs = serde_json::from_value(args.clone())?;
    for (axis, limit) in [
        ("max_children", a.max_children),
        ("max_depth", a.max_depth),
        ("max_tools", a.max_tools),
    ] {
        if limit.is_some_and(|l| l < 0) {
            return Err(McpError::InvalidParams(format!("{axis} must be >= 0")));
        }
    }
    store
        .set_spawn_budget(auth.workspace_id, a.max_children, a.max_depth, a.max_tools)
        .await?;
    Ok(content_json(&json!({
        "max_children": a.max_children,
        "max_depth": a.max_depth,
        "max_tools": a.max_tools,
    })))
}

/// The caller's workspace spawn budget; every axis is null when unset
/// (unlimited) (Cluster 376.4). `workspace:read`.
pub(super) async fn get_spawn_budget(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    _args: &Value,
) -> Result<Value, McpError> {
    let budget = store.get_spawn_budget(auth.workspace_id).await?;
    Ok(content_json(&json!({
        "max_children": budget.as_ref().and_then(|b| b.max_children),
        "max_depth": budget.as_ref().and_then(|b| b.max_depth),
        "max_tools": budget.as_ref().and_then(|b| b.max_tools),
    })))
}

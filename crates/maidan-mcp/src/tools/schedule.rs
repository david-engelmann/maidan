//! Task-schedule MCP tools (Cluster 229): an agent creates + inspects its own
//! recurring/one-shot schedules. The REST twin is Cluster 228. Writes are gated
//! `workspace:write` + target-channel access; the list is `workspace:read`,
//! filtered to channels the caller can access.

use std::sync::Arc;

use maidan_auth::AuthContext;
use maidan_router::resolve_channel_context;
use maidan_store::Store;
use maidan_types::*;
use serde::Deserialize;
use serde_json::Value;

use super::content_json;
use crate::error::McpError;

#[derive(Deserialize)]
struct CreateScheduleArgs {
    channel_id: uuid::Uuid,
    title: String,
    #[serde(default)]
    interval_secs: Option<i64>,
    #[serde(default)]
    first_run_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Create a task schedule (Cluster 229). When due, the sweeper materializes a
/// thread titled `title` in `channel_id`. Channel access is enforced pre-dispatch
/// (the `channel_id` arg); the schedule is owned by the caller
/// (`created_by = auth.member_id`) and scoped to the caller's workspace.
pub(super) async fn create_task_schedule(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: CreateScheduleArgs = serde_json::from_value(args.clone())?;
    if a.title.trim().is_empty() {
        return Err(McpError::InvalidParams("title must not be empty".into()));
    }
    if let Some(secs) = a.interval_secs {
        if secs <= 0 {
            return Err(McpError::InvalidParams(
                "interval_secs must be positive (omit for a one-shot)".into(),
            ));
        }
    }
    let channel_id = ChannelId(a.channel_id);
    let ctx = resolve_channel_context(store.as_ref(), channel_id)
        .await
        .map_err(|e| McpError::InvalidParams(e.to_string()))?;
    if !auth.bypass && ctx.workspace_id != auth.workspace_id {
        return Err(McpError::InvalidParams(
            "channel is not in the caller's workspace".into(),
        ));
    }
    let schedule = store
        .create_task_schedule(NewTaskSchedule {
            workspace_id: ctx.workspace_id,
            channel_id,
            title: a.title.trim().to_string(),
            interval_secs: a.interval_secs,
            next_run_at: a.first_run_at.unwrap_or_else(chrono::Utc::now),
            created_by: auth.member_id,
        })
        .await?;
    Ok(content_json(&schedule))
}

/// List the caller's workspace's task schedules (Cluster 229), filtered to the
/// channels the caller can access. A workspace-scoped aggregate read, so the
/// pre-dispatch channel gate can't cover it — the handler filters like
/// `list_assigned_threads`.
pub(super) async fn list_task_schedules(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    _args: &Value,
) -> Result<Value, McpError> {
    let schedules = store.list_task_schedules(auth.workspace_id).await?;
    if auth.bypass {
        return Ok(content_json(&schedules));
    }
    let mut visible = Vec::with_capacity(schedules.len());
    for s in schedules {
        if maidan_auth::can_access_channel(store.as_ref(), auth, s.channel_id).await? {
            visible.push(s);
        }
    }
    Ok(content_json(&visible))
}

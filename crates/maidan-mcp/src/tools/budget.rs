//! Budget-envelope tool handlers (Cluster 358, T1/T5): set/read a thread's
//! budget, report usage (stopping the run on exceed), and read a channel's DLQ.

use std::sync::Arc;

use maidan_store::Store;
use maidan_types::{BudgetLimits, ChannelId, ThreadId, UsageDelta};
use serde::Deserialize;
use serde_json::Value;

use super::content_json;
use crate::error::McpError;

#[derive(Deserialize)]
struct SetBudgetArgs {
    thread_id: uuid::Uuid,
    #[serde(default)]
    max_tokens: Option<i64>,
    #[serde(default)]
    max_usd_micros: Option<i64>,
    #[serde(default)]
    max_turns: Option<i64>,
    #[serde(default)]
    max_wall_secs: Option<i64>,
}

/// Set (upsert) a thread's budget maxima (Cluster 358, the MCP twin of
/// `PUT /threads/:id/budget`). Accumulated usage is preserved. Thread access is
/// enforced pre-dispatch.
pub(super) async fn set_thread_budget(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: SetBudgetArgs = serde_json::from_value(args.clone())?;
    let budget = store
        .set_thread_budget(
            ThreadId(a.thread_id),
            BudgetLimits {
                max_tokens: a.max_tokens,
                max_usd_micros: a.max_usd_micros,
                max_turns: a.max_turns,
                max_wall_secs: a.max_wall_secs,
            },
        )
        .await?;
    Ok(content_json(&budget))
}

#[derive(Deserialize)]
struct ThreadIdArg {
    thread_id: uuid::Uuid,
}

/// A thread's budget, or null until one is set / usage is first reported.
pub(super) async fn get_thread_budget(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ThreadIdArg = serde_json::from_value(args.clone())?;
    let budget = store.get_thread_budget(ThreadId(a.thread_id)).await?;
    Ok(content_json(&budget))
}

#[derive(Deserialize)]
struct ReportUsageArgs {
    thread_id: uuid::Uuid,
    #[serde(default)]
    tokens: i64,
    #[serde(default)]
    usd_micros: i64,
    #[serde(default)]
    turns: i64,
}

/// Report incremental usage against a thread's budget (Cluster 358, the MCP twin
/// of `POST /threads/:id/usage`) — the claim-holder's heartbeat. If it pushes a
/// claimed thread over budget the run is STOPPED (claim released, `ClaimFailed`
/// emitted, DLQ entry recorded); the returned `stopped`/`reason` say so. Thread
/// access is enforced pre-dispatch.
pub(super) async fn report_usage(
    server: &crate::server::McpServer,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ReportUsageArgs = serde_json::from_value(args.clone())?;
    let (report, stored) = server
        .store
        .report_thread_usage(
            ThreadId(a.thread_id),
            UsageDelta {
                tokens: a.tokens,
                usd_micros: a.usd_micros,
                turns: a.turns,
            },
        )
        .await?;
    if let Some(stored) = stored {
        server.publish_stored(&stored).await;
    }
    Ok(content_json(&report))
}

#[derive(Deserialize)]
struct ListDlqArgs {
    channel_id: uuid::Uuid,
    #[serde(default)]
    limit: Option<i64>,
}

/// A channel's agent-work dead-letter queue (Cluster 358) — runs stopped for
/// exceeding their budget, newest first. Channel access is enforced pre-dispatch.
pub(super) async fn list_dlq(store: &Arc<dyn Store>, args: &Value) -> Result<Value, McpError> {
    let a: ListDlqArgs = serde_json::from_value(args.clone())?;
    let limit = a.limit.unwrap_or(50).clamp(1, 200);
    let entries = store
        .list_channel_dlq(ChannelId(a.channel_id), limit)
        .await?;
    Ok(content_json(&entries))
}

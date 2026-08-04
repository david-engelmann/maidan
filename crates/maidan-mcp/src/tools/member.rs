//! Member inbox + mention tool handlers (Cluster 149). Lets an MCP-only agent
//! discover it was @mentioned — the read side previously reachable over HTTP
//! only (`/members/:id/mentions`, `/members/:id/inbox`).

use std::sync::Arc;

use chrono::{DateTime, Utc};
use maidan_store::Store;
use maidan_types::*;
use serde::Deserialize;
use serde_json::Value;

use super::content_json;
use crate::error::McpError;

/// Default + max list size, mirroring the context tools' clamp.
fn clamp_limit(limit: Option<i64>) -> i64 {
    limit.unwrap_or(50).clamp(1, 500)
}

#[derive(Deserialize)]
struct MemberLimitArgs {
    member_id: uuid::Uuid,
    #[serde(default)]
    limit: Option<i64>,
}

pub(super) async fn list_mentions(store: &Arc<dyn Store>, args: &Value) -> Result<Value, McpError> {
    let a: MemberLimitArgs = serde_json::from_value(args.clone())?;
    let mentions = store
        .list_mentions_for_member(MemberId(a.member_id), clamp_limit(a.limit))
        .await?;
    Ok(content_json(&mentions))
}

pub(super) async fn get_inbox(store: &Arc<dyn Store>, args: &Value) -> Result<Value, McpError> {
    let a: MemberLimitArgs = serde_json::from_value(args.clone())?;
    let inbox = store
        .list_member_inbox(MemberId(a.member_id), clamp_limit(a.limit))
        .await?;
    Ok(content_json(&inbox))
}

#[derive(Deserialize)]
struct MarkInboxReadArgs {
    member_id: uuid::Uuid,
    /// Advance the member's read cursor through this instant (RFC 3339).
    read_through: DateTime<Utc>,
}

pub(super) async fn mark_inbox_read(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: MarkInboxReadArgs = serde_json::from_value(args.clone())?;
    store
        .advance_inbox_last_read_at(MemberId(a.member_id), a.read_through)
        .await?;
    let inbox = store.list_member_inbox(MemberId(a.member_id), 50).await?;
    Ok(content_json(&inbox))
}

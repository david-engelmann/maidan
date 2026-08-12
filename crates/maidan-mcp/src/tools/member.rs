//! Member inbox + mention tool handlers (Cluster 149). Lets an MCP-only agent
//! discover it was @mentioned — the read side previously reachable over HTTP
//! only (`/members/:id/mentions`, `/members/:id/inbox`). Cluster 196 adds
//! `wait_for_mention`, a blocking long-poll over the event bus.

use std::{collections::HashSet, sync::Arc, time::Duration};

use chrono::{DateTime, Utc};
use futures::StreamExt;
use maidan_auth::AuthContext;
use maidan_bus::BusItem;
use maidan_store::Store;
use maidan_types::*;
use serde::Deserialize;
use serde_json::Value;

use super::content_json;
use crate::error::McpError;

/// Default long-poll window, and the ceiling a caller may request.
const DEFAULT_WAIT_MS: i64 = 30_000;
const MAX_WAIT_MS: i64 = 300_000;

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

#[derive(Deserialize)]
struct WaitForMentionArgs {
    member_id: uuid::Uuid,
    /// Long-poll window in milliseconds (default 30 000, clamped to 1 000–300 000).
    #[serde(default)]
    timeout_ms: Option<i64>,
}

/// Block until `member_id` is next @mentioned, or the timeout lapses (Cluster
/// 196). Subscribes to the event bus filtered to this member's
/// `MentionRecorded` events and returns the first one whose thread the caller
/// can access; returns `null` on timeout. This is a **live** primitive — it only
/// sees mentions recorded *after* the call subscribes, so drain existing ones
/// with `get_inbox`/`list_mentions` first (the `GET /mcp/stream` SSE transport
/// is the at-least-once, resumable alternative when a missed mention is
/// unacceptable).
pub(super) async fn wait_for_mention(
    server: &crate::server::McpServer,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: WaitForMentionArgs = serde_json::from_value(args.clone())?;
    let member_id = MemberId(a.member_id);
    let Some(bus) = server.event_bus.as_ref() else {
        return Err(McpError::InvalidParams(
            "wait_for_mention requires an event bus".into(),
        ));
    };
    let wait = a
        .timeout_ms
        .unwrap_or(DEFAULT_WAIT_MS)
        .clamp(1, MAX_WAIT_MS);

    let filter = EventFilter {
        member_id: Some(member_id),
        kinds: Some(HashSet::from([EventKind::MentionRecorded])),
        ..EventFilter::default()
    };
    let mut stream = bus
        .subscribe(filter)
        .await
        .map_err(|e| McpError::Internal(e.to_string()))?;

    let store = server.store.as_ref();
    let deadline = tokio::time::Instant::now() + Duration::from_millis(wait as u64);
    loop {
        let item = match tokio::time::timeout_at(deadline, stream.next()).await {
            // Timed out or the bus closed → no mention arrived in the window.
            Err(_) | Ok(None) => return Ok(content_json(&Value::Null)),
            Ok(Some(item)) => item,
        };
        // A lag marker means the buffer overflowed; keep waiting rather than
        // reporting a false timeout (bounded by the same deadline).
        let BusItem::Event(envelope) = item else {
            continue;
        };
        let thread_id = envelope.event.thread_id();
        // The mention is addressed to this member, but if it lives in a thread
        // the caller can't access, don't reveal it — keep waiting.
        if !auth.bypass {
            if let Some(tid) = thread_id {
                if !maidan_auth::can_access_thread(store, auth, tid).await? {
                    continue;
                }
            }
        }
        return Ok(content_json(&envelope.event));
    }
}

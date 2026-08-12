//! Thread listing + sampling-backed summarization tool handlers.

use std::sync::Arc;

use chrono::Utc;
use maidan_auth::AuthContext;
use maidan_router::resolve_thread_context;
use maidan_store::Store;
use maidan_types::*;
use serde::Deserialize;
use serde_json::{json, Value};

use super::content_json;
use crate::error::McpError;

#[derive(Deserialize)]
struct ListThreadsArgs {
    channel_id: uuid::Uuid,
}

pub(super) async fn list_threads(store: &Arc<dyn Store>, args: &Value) -> Result<Value, McpError> {
    let a: ListThreadsArgs = serde_json::from_value(args.clone())?;
    let threads = store.list_threads(ChannelId(a.channel_id)).await?;
    Ok(content_json(&threads))
}

#[derive(Deserialize)]
struct ToolTranscriptArgs {
    thread_id: uuid::Uuid,
    /// Max messages to scan (default 200, clamped 1..=500).
    #[serde(default)]
    limit: Option<i64>,
}

/// A thread's tool-call transcript (Cluster 197): every `ToolUse` block across
/// the thread's messages, each correlated with its `ToolResult` by id. A
/// token-lean projection — `Text`/`Code` blocks and `body` are dropped. Channel
/// access is enforced pre-dispatch (the `thread_id` arg).
pub(super) async fn get_tool_transcript(
    store: &Arc<dyn Store>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ToolTranscriptArgs = serde_json::from_value(args.clone())?;
    let thread_id = ThreadId(a.thread_id);
    let limit = a.limit.unwrap_or(200).clamp(1, 500);
    let messages = store.list_messages(thread_id, limit).await?;
    Ok(content_json(&tool_transcript(thread_id, &messages)))
}

#[derive(Deserialize)]
struct AssignThreadArgs {
    thread_id: uuid::Uuid,
    actor_id: uuid::Uuid,
    assignee_id: uuid::Uuid,
    /// Optional handoff note for the assignee (Cluster 195).
    #[serde(default)]
    note: Option<String>,
}

#[derive(Deserialize)]
struct ClaimThreadArgs {
    thread_id: uuid::Uuid,
    member_id: uuid::Uuid,
}

#[derive(Deserialize)]
struct UnassignThreadArgs {
    thread_id: uuid::Uuid,
    actor_id: uuid::Uuid,
}

/// Emit a `ThreadAssignmentChanged` event for an assignment mutation
/// (Cluster 171). No-op when the bus is unconfigured.
async fn publish_assignment(
    server: &crate::server::McpServer,
    thread: &Thread,
    actor_id: MemberId,
    previous_assignee_id: Option<MemberId>,
    note: Option<String>,
) -> Result<(), McpError> {
    if server.event_bus.is_none() {
        return Ok(());
    }
    let ctx = resolve_thread_context(server.store.as_ref(), thread.id)
        .await
        .map_err(|e| McpError::InvalidParams(e.to_string()))?;
    server
        .publish_event(Event::ThreadAssignmentChanged {
            occurred_at: Utc::now(),
            workspace_id: ctx.workspace_id,
            channel_id: ctx.channel_id,
            thread_id: thread.id,
            actor_id,
            previous_assignee_id,
            assignee_id: thread.assignee_id,
            note,
            thread: thread.clone(),
        })
        .await;
    Ok(())
}

pub(super) async fn assign_thread(
    server: &crate::server::McpServer,
    args: &Value,
) -> Result<Value, McpError> {
    let a: AssignThreadArgs = serde_json::from_value(args.clone())?;
    let thread_id = ThreadId(a.thread_id);
    let previous = server.store.get_thread(thread_id).await?.assignee_id;
    let thread = server
        .store
        .assign_thread(thread_id, MemberId(a.assignee_id))
        .await?;
    publish_assignment(server, &thread, MemberId(a.actor_id), previous, a.note).await?;
    Ok(content_json(&thread))
}

pub(super) async fn claim_thread(
    server: &crate::server::McpServer,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ClaimThreadArgs = serde_json::from_value(args.clone())?;
    let thread_id = ThreadId(a.thread_id);
    let member_id = MemberId(a.member_id);
    let result = server.store.claim_thread(thread_id, member_id).await?;
    if result.claimed {
        publish_assignment(server, &result.thread, member_id, None, None).await?;
    }
    Ok(content_json(&result))
}

pub(super) async fn unassign_thread(
    server: &crate::server::McpServer,
    args: &Value,
) -> Result<Value, McpError> {
    let a: UnassignThreadArgs = serde_json::from_value(args.clone())?;
    let thread_id = ThreadId(a.thread_id);
    let previous = server.store.get_thread(thread_id).await?.assignee_id;
    let thread = server.store.unassign_thread(thread_id).await?;
    publish_assignment(server, &thread, MemberId(a.actor_id), previous, None).await?;
    Ok(content_json(&thread))
}

#[derive(Deserialize)]
struct ListAssignedThreadsArgs {
    member_id: uuid::Uuid,
}

/// A member's assigned-thread queue (Cluster 191). A member-scoped aggregate
/// read: the pre-dispatch channel gate can't cover a `member_id` arg, so this
/// filters the result to threads the caller can access (like `search_messages`).
pub(super) async fn list_assigned_threads(
    store: &Arc<dyn Store>,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ListAssignedThreadsArgs = serde_json::from_value(args.clone())?;
    let member_id = MemberId(a.member_id);
    let member = store.get_member(member_id).await?;
    let threads = store
        .list_assigned_threads(member.workspace_id, member_id)
        .await?;
    if auth.bypass {
        return Ok(content_json(&threads));
    }
    let mut visible = Vec::with_capacity(threads.len());
    for t in threads {
        if maidan_auth::can_access_thread(store.as_ref(), auth, t.id).await? {
            visible.push(t);
        }
    }
    Ok(content_json(&visible))
}

#[derive(Deserialize)]
struct ClaimNextThreadArgs {
    channel_id: uuid::Uuid,
    member_id: uuid::Uuid,
    /// Optional lease deadline in seconds; the claim is reclaimable after it
    /// lapses (Cluster 192). Omit for a durable claim.
    #[serde(default)]
    lease_secs: Option<i64>,
}

/// Atomically claim the oldest claimable thread in a channel (Cluster 191/192).
/// Channel access is enforced pre-dispatch (the `channel_id` arg). Returns the
/// claimed thread, or `null` when there is no claimable work.
pub(super) async fn claim_next_thread(
    server: &crate::server::McpServer,
    args: &Value,
) -> Result<Value, McpError> {
    let a: ClaimNextThreadArgs = serde_json::from_value(args.clone())?;
    let member_id = MemberId(a.member_id);
    let claimed = server
        .store
        .claim_next_thread(ChannelId(a.channel_id), member_id, a.lease_secs)
        .await?;
    if let Some(thread) = &claimed {
        publish_assignment(server, thread, member_id, None, None).await?;
    }
    Ok(content_json(&claimed))
}

#[derive(Deserialize)]
struct RenewClaimArgs {
    thread_id: uuid::Uuid,
    member_id: uuid::Uuid,
    lease_secs: i64,
}

/// Extend a claimed thread's lease (heartbeat), only for the current assignee
/// (Cluster 192). Thread access is enforced pre-dispatch (the `thread_id` arg).
pub(super) async fn renew_claim(
    server: &crate::server::McpServer,
    args: &Value,
) -> Result<Value, McpError> {
    let a: RenewClaimArgs = serde_json::from_value(args.clone())?;
    let thread = server
        .store
        .renew_claim(ThreadId(a.thread_id), MemberId(a.member_id), a.lease_secs)
        .await?;
    Ok(content_json(&thread))
}

#[derive(Deserialize)]
struct SummarizeThreadArgs {
    thread_id: uuid::Uuid,
    #[serde(default = "default_summary_limit")]
    limit: i64,
    /// Optional steer for the summary (defaults to a plain "summarize concisely").
    #[serde(default)]
    instructions: Option<String>,
}

fn default_summary_limit() -> i64 {
    50
}

/// Summarize a thread by asking the *client* to sample an LLM: the server issues
/// a `sampling/createMessage` request over the session's canonical GET stream
/// (Cluster 154 delivery) and returns the client's completion. This is the first
/// organic caller of [`crate::server::McpServer::request_client`] — it needs a
/// streamable session whose client declared the `sampling` capability.
pub(super) async fn summarize_thread(
    server: &crate::server::McpServer,
    session_id: Option<&str>,
    args: &Value,
) -> Result<Value, McpError> {
    let a: SummarizeThreadArgs = serde_json::from_value(args.clone())?;
    let session = session_id.ok_or_else(|| {
        McpError::InvalidParams(
            "summarize_thread requires a streamable session (open GET /mcp/streamable) whose \
             client supports sampling"
                .into(),
        )
    })?;

    let messages = server
        .store
        .list_messages(ThreadId(a.thread_id), a.limit.clamp(1, 500))
        .await?;
    if messages.is_empty() {
        return Err(McpError::InvalidParams(
            "thread has no messages to summarize".into(),
        ));
    }
    let transcript = messages
        .iter()
        .map(|m| format!("{}: {}", m.author_id.0, m.body))
        .collect::<Vec<_>>()
        .join("\n");
    let instructions = a
        .instructions
        .as_deref()
        .unwrap_or("Summarize this thread concisely.");

    // MCP `sampling/createMessage` shape: the client runs the completion and
    // returns the message. The server never sees an API key — the sampling
    // happens on the client's side of the connection.
    let params = json!({
        "messages": [{
            "role": "user",
            "content": { "type": "text", "text": format!("{instructions}\n\n---\n{transcript}") }
        }],
        "maxTokens": 512,
    });
    let result = server
        .request_client(session, "sampling/createMessage", params)
        .await?;
    Ok(content_json(&result))
}

//! Thread listing + sampling-backed summarization tool handlers.

use std::sync::Arc;

use chrono::Utc;
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
struct AssignThreadArgs {
    thread_id: uuid::Uuid,
    actor_id: uuid::Uuid,
    assignee_id: uuid::Uuid,
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
    publish_assignment(server, &thread, MemberId(a.actor_id), previous).await?;
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
        publish_assignment(server, &result.thread, member_id, None).await?;
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
    publish_assignment(server, &thread, MemberId(a.actor_id), previous).await?;
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

//! Thread listing + sampling-backed summarization tool handlers.

use std::sync::Arc;

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

//! MCP context export (Cluster 74) — mirrors HTTP context packs.

use maidan_store::Store;
use maidan_types::*;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::error::McpError;

#[derive(Debug, Deserialize)]
struct ThreadContextArgs {
    thread_id: Uuid,
    #[serde(default = "default_message_limit")]
    message_limit: i64,
    #[serde(default = "default_transition_limit")]
    transition_limit: i64,
}

#[derive(Debug, Deserialize)]
struct WorkspaceContextArgs {
    workspace_id: Uuid,
    #[serde(default = "default_thread_limit")]
    thread_limit: i64,
    #[serde(default = "default_message_limit")]
    message_limit: i64,
    #[serde(default = "default_transition_limit")]
    transition_limit: i64,
}

fn default_message_limit() -> i64 {
    100
}
fn default_transition_limit() -> i64 {
    50
}
fn default_thread_limit() -> i64 {
    10
}

pub async fn get_thread_context(store: &dyn Store, args: &Value) -> Result<Value, McpError> {
    let a: ThreadContextArgs =
        serde_json::from_value(args.clone()).map_err(|e| McpError::InvalidParams(e.to_string()))?;
    let thread_id = ThreadId(a.thread_id);
    let thread = store.get_thread(thread_id).await?;
    if thread.tombstoned_at.is_some() {
        return Err(McpError::InvalidParams("thread is tombstoned".into()));
    }
    let _thread_state = thread.state;
    let channel = store.get_channel(thread.channel_id).await?;
    let messages = store
        .list_messages(thread_id, a.message_limit.clamp(1, 500))
        .await?;
    let transitions = store
        .list_thread_transitions(thread_id, a.transition_limit.clamp(1, 200))
        .await?;
    let mut references = store
        .list_references_from(RefSide::Thread, thread_id.0)
        .await?;
    for message in &messages {
        let mut from_message = store
            .list_references_from(RefSide::Message, message.id.0)
            .await?;
        references.append(&mut from_message);
    }
    references.sort_by_key(|r| r.created_at);
    references.dedup_by_key(|r| r.id);

    let mut message_edits = Vec::new();
    for message in &messages {
        let mut edits = store.list_message_edits(message.id, 20).await?;
        message_edits.append(&mut edits);
    }

    Ok(json!({
        "workspace_id": channel.workspace_id.0,
        "channel_id": thread.channel_id.0,
        "thread": thread,
        "messages": messages,
        "message_edits": message_edits,
        "references": references,
        "fsm": {
            "state": thread.state,
            "transitions": transitions,
        }
    }))
}

pub async fn get_workspace_context(store: &dyn Store, args: &Value) -> Result<Value, McpError> {
    let a: WorkspaceContextArgs =
        serde_json::from_value(args.clone()).map_err(|e| McpError::InvalidParams(e.to_string()))?;
    let workspace_id = WorkspaceId(a.workspace_id);
    let workspace = store.get_workspace(workspace_id).await?;
    let channels = store.list_channels(workspace_id).await?;
    let thread_limit = a.thread_limit.clamp(1, 50);
    let mut threads = Vec::new();
    for channel in &channels {
        if threads.len() as i64 >= thread_limit {
            break;
        }
        for thread in store.list_threads(channel.id).await? {
            if threads.len() as i64 >= thread_limit {
                break;
            }
            if thread.tombstoned_at.is_some() {
                continue;
            }
            let packed = get_thread_context(
                store,
                &json!({
                    "thread_id": thread.id.0,
                    "message_limit": a.message_limit,
                    "transition_limit": a.transition_limit,
                }),
            )
            .await?;
            threads.push(packed);
        }
    }
    Ok(json!({
        "workspace": workspace,
        "channels": channels,
        "threads": threads,
    }))
}

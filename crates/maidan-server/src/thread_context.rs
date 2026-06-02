//! Assemble agent-ready thread context for prompt packing.

use std::collections::HashSet;

use maidan_store::Store;
use maidan_types::*;

use crate::error::ApiError;

#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct ThreadFsmContext {
    pub state: ThreadState,
    pub transitions: Vec<ThreadTransition>,
}

#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct ThreadContext {
    pub workspace_id: WorkspaceId,
    pub channel_id: ChannelId,
    pub thread: Thread,
    pub messages: Vec<Message>,
    pub message_edits: Vec<MessageEdit>,
    pub references: Vec<Reference>,
    pub artifacts: Vec<Artifact>,
    pub fsm: ThreadFsmContext,
    /// Present when more messages exist (`message_id` cursor for the next page).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_message_cursor: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct WorkspaceContext {
    pub workspace: Workspace,
    pub channels: Vec<Channel>,
    pub threads: Vec<ThreadContext>,
    /// Present when more threads exist (`thread_id` cursor for the next page).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_thread_cursor: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub struct ThreadContextLimits {
    pub message_limit: i64,
    pub transition_limit: i64,
    pub message_cursor: Option<MessageId>,
}

impl Default for ThreadContextLimits {
    fn default() -> Self {
        Self {
            message_limit: 100,
            transition_limit: 50,
            message_cursor: None,
        }
    }
}

pub async fn build_thread_context(
    store: &dyn Store,
    thread_id: ThreadId,
    limits: ThreadContextLimits,
) -> Result<ThreadContext, ApiError> {
    let thread = store.get_thread(thread_id).await?;
    if thread.tombstoned_at.is_some() {
        return Err(ApiError::NotFound);
    }
    let channel = store.get_channel(thread.channel_id).await?;
    let workspace_id = channel.workspace_id;

    let page_limit = limits.message_limit.clamp(1, 500);
    let messages = store
        .list_messages_after(thread_id, limits.message_cursor, page_limit + 1)
        .await?;
    let next_message_cursor = if messages.len() as i64 > page_limit {
        messages
            .get(page_limit as usize - 1)
            .map(|m| m.id.0.to_string())
    } else {
        None
    };
    let messages: Vec<Message> = messages.into_iter().take(page_limit as usize).collect();
    let transitions = store
        .list_thread_transitions(thread_id, limits.transition_limit)
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

    let mut artifact_shas = HashSet::new();
    for message in &messages {
        for sha in artifact_shas_from_metadata(&message.metadata) {
            artifact_shas.insert(sha);
        }
    }

    let mut artifacts = Vec::new();
    for sha in artifact_shas {
        match store.get_artifact_by_sha(&sha).await {
            Ok(artifact) if artifact.tombstoned_at.is_none() => artifacts.push(artifact),
            Ok(_) => {}
            Err(_) => {}
        }
    }
    artifacts.sort_by_key(|a| a.created_at);

    let mut message_edits = Vec::new();
    for message in &messages {
        let mut edits = store.list_message_edits(message.id, 20).await?;
        message_edits.append(&mut edits);
    }

    Ok(ThreadContext {
        workspace_id,
        channel_id: thread.channel_id,
        thread: thread.clone(),
        messages,
        message_edits,
        references,
        artifacts,
        fsm: ThreadFsmContext {
            state: thread.state,
            transitions,
        },
        next_message_cursor,
    })
}

pub async fn build_workspace_context(
    store: &dyn Store,
    workspace_id: WorkspaceId,
    thread_limit: i64,
    thread_cursor: Option<ThreadId>,
    limits: ThreadContextLimits,
) -> Result<WorkspaceContext, ApiError> {
    let workspace = store.get_workspace(workspace_id).await?;
    let channels = store.list_channels(workspace_id).await?;
    let page_limit = thread_limit.clamp(1, 50);
    let mut ordered_threads = Vec::new();
    for channel in &channels {
        let channel_threads = store.list_threads(channel.id).await?;
        for thread in channel_threads {
            if thread.tombstoned_at.is_some() {
                continue;
            }
            ordered_threads.push(thread);
        }
    }
    ordered_threads.sort_by(|a, b| {
        a.created_at
            .cmp(&b.created_at)
            .then_with(|| a.id.0.cmp(&b.id.0))
    });
    let start = thread_cursor.map_or(0, |cursor| {
        ordered_threads
            .iter()
            .position(|t| t.id == cursor)
            .map(|i| i + 1)
            .unwrap_or(ordered_threads.len())
    });
    let slice = ordered_threads
        .into_iter()
        .skip(start)
        .take(page_limit as usize + 1)
        .collect::<Vec<_>>();
    let next_thread_cursor = if slice.len() > page_limit as usize {
        slice
            .get(page_limit as usize - 1)
            .map(|t| t.id.0.to_string())
    } else {
        None
    };
    let mut threads = Vec::new();
    for thread in slice.into_iter().take(page_limit as usize) {
        threads.push(build_thread_context(store, thread.id, limits).await?);
    }
    Ok(WorkspaceContext {
        workspace,
        channels,
        threads,
        next_thread_cursor,
    })
}

fn artifact_shas_from_metadata(metadata: &serde_json::Value) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(s) = metadata.get("artifact_sha256").and_then(|v| v.as_str()) {
        out.push(s.to_string());
    }
    if let Some(s) = metadata.get("sha256").and_then(|v| v.as_str()) {
        out.push(s.to_string());
    }
    if let Some(arr) = metadata.get("artifacts").and_then(|v| v.as_array()) {
        for item in arr {
            if let Some(s) = item.as_str() {
                out.push(s.to_string());
            } else if let Some(sha) = item.get("sha256").and_then(|v| v.as_str()) {
                out.push(sha.to_string());
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn metadata_extractor_collects_sha_fields() {
        let meta = json!({
            "artifact_sha256": "aa".repeat(32),
            "artifacts": [{"sha256": "bb".repeat(32)}]
        });
        let shas = artifact_shas_from_metadata(&meta);
        assert_eq!(shas.len(), 2);
    }
}

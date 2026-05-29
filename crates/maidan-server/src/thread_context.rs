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
    pub references: Vec<Reference>,
    pub artifacts: Vec<Artifact>,
    pub fsm: ThreadFsmContext,
}

#[derive(Debug, Clone, Copy)]
pub struct ThreadContextLimits {
    pub message_limit: i64,
    pub transition_limit: i64,
}

impl Default for ThreadContextLimits {
    fn default() -> Self {
        Self {
            message_limit: 100,
            transition_limit: 50,
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

    let messages = store.list_messages(thread_id, limits.message_limit).await?;
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

    Ok(ThreadContext {
        workspace_id,
        channel_id: thread.channel_id,
        thread: thread.clone(),
        messages,
        references,
        artifacts,
        fsm: ThreadFsmContext {
            state: thread.state,
            transitions,
        },
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

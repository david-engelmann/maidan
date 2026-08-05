//! Assemble agent-ready thread context for prompt packing.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use maidan_store::Store;
use maidan_types::*;

use crate::error::ApiError;

#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct ThreadFsmContext {
    pub state: ThreadState,
    pub transitions: Vec<ThreadTransition>,
}

/// A context-pack edit record. The `body_before`/`body_after` diff copies are
/// the single largest token cost in a pack, so they are omitted unless the
/// caller asks for them (`include_edits=true`); the who/when/which-message
/// signal is always present. See `crates/maidan-mcp/src/context.rs` for the
/// MCP-side twin of this behavior.
#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct MessageEditView {
    pub id: i64,
    pub message_id: MessageId,
    pub editor_id: MemberId,
    pub edited_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_before: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body_after: Option<String>,
}

impl MessageEditView {
    fn from_edit(edit: MessageEdit, include_bodies: bool) -> Self {
        let (body_before, body_after) = if include_bodies {
            (Some(edit.body_before), Some(edit.body_after))
        } else {
            (None, None)
        };
        Self {
            id: edit.id,
            message_id: edit.message_id,
            editor_id: edit.editor_id,
            edited_at: edit.edited_at,
            body_before,
            body_after,
        }
    }
}

#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct ThreadContext {
    pub workspace_id: WorkspaceId,
    pub channel_id: ChannelId,
    pub thread: Thread,
    pub messages: Vec<Message>,
    pub message_edits: Vec<MessageEditView>,
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
    /// Include full `body_before`/`body_after` on each edit. Default `false`:
    /// edits carry metadata only. The single biggest token lever on a pack.
    pub include_edits: bool,
}

impl Default for ThreadContextLimits {
    fn default() -> Self {
        Self {
            message_limit: 100,
            transition_limit: 50,
            message_cursor: None,
            include_edits: false,
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

    // One reference read for the thread + one batched read across all messages,
    // replacing the per-message N+1. Ordering (created_at) is unchanged.
    let message_src_ids: Vec<uuid::Uuid> = messages.iter().map(|m| m.id.0).collect();
    let mut references = store
        .list_references_from(RefSide::Thread, thread_id.0)
        .await?;
    let mut from_messages = store
        .list_references_from_many(RefSide::Message, &message_src_ids)
        .await?;
    references.append(&mut from_messages);
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

    // Batched edit read (≤20 per message), then re-grouped into the original
    // per-message order so the response ordering contract is unchanged.
    let message_ids: Vec<MessageId> = messages.iter().map(|m| m.id).collect();
    let message_pos: HashMap<MessageId, usize> = message_ids
        .iter()
        .enumerate()
        .map(|(i, id)| (*id, i))
        .collect();
    let mut message_edits = store
        .list_message_edits_for_messages(&message_ids, 20)
        .await?;
    message_edits.sort_by_key(|e| {
        (
            message_pos
                .get(&e.message_id)
                .copied()
                .unwrap_or(usize::MAX),
            e.edited_at,
            e.id,
        )
    });
    let message_edits: Vec<MessageEditView> = message_edits
        .into_iter()
        .map(|e| MessageEditView::from_edit(e, limits.include_edits))
        .collect();

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
    // One keyset page from SQL (ordered `(created_at, id)`, tombstoned filtered),
    // fetching one extra row to detect a next page — no longer loads every
    // workspace thread to slice it in memory.
    let mut page = store
        .page_threads_for_workspace(workspace_id, thread_cursor, page_limit + 1)
        .await?;
    let has_more = page.len() > page_limit as usize;
    page.truncate(page_limit as usize);
    let next_thread_cursor = if has_more {
        page.last().map(|t| t.id.0.to_string())
    } else {
        None
    };
    let mut threads = Vec::new();
    for thread in page {
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

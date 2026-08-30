//! Assemble agent-ready thread context for prompt packing.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use futures::stream::{self, StreamExt, TryStreamExt};
use maidan_store::Store;
use maidan_types::*;

use crate::error::ApiError;

/// Max concurrent per-thread context builds inside a workspace-context pack
/// (Cluster 199). Each build is ~7 store round-trips; bounding the fan-out keeps
/// a single request from saturating the connection pool while still collapsing
/// the sequential per-thread latency of a page.
const CONTEXT_THREAD_CONCURRENCY: usize = 8;

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
    /// The workspace glossary (Cluster 323) — canonical term definitions grounding
    /// the pack in shared vocabulary. Omitted when empty or when the caller opts
    /// out (`include_glossary=false`). On a workspace-context pack this rides the
    /// top-level `WorkspaceContext.glossary` instead (not repeated per thread).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub glossary: Vec<GlossaryTerm>,
    /// Present when more messages exist (`message_id` cursor for the next page).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_message_cursor: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize, utoipa::ToSchema)]
pub struct WorkspaceContext {
    pub workspace: Workspace,
    pub channels: Vec<Channel>,
    pub threads: Vec<ThreadContext>,
    /// The workspace glossary (Cluster 323), carried once here rather than on each
    /// nested thread pack. Omitted when empty or when `include_glossary=false`.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub glossary: Vec<GlossaryTerm>,
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
    /// Attach the workspace glossary to the pack (Cluster 323). Default `true`.
    /// A workspace-context build sets this `false` on its nested thread builds so
    /// the glossary rides the top level once, not per thread.
    pub include_glossary: bool,
    /// As-of context replay (Cluster 326): when set, reconstruct the thread as it
    /// stood at this event-log id — deterministic over the immutable log, no fresh
    /// search. `None` = the live pack.
    pub as_of: Option<i64>,
}

impl Default for ThreadContextLimits {
    fn default() -> Self {
        Self {
            message_limit: 100,
            transition_limit: 50,
            message_cursor: None,
            include_edits: false,
            include_glossary: true,
            as_of: None,
        }
    }
}

pub async fn build_thread_context(
    store: &dyn Store,
    thread_id: ThreadId,
    limits: ThreadContextLimits,
) -> Result<ThreadContext, ApiError> {
    if let Some(as_of) = limits.as_of {
        return build_thread_context_as_of(store, thread_id, as_of, limits).await;
    }
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

    let glossary = if limits.include_glossary {
        store.list_glossary_terms(workspace_id).await?
    } else {
        Vec::new()
    };

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
        glossary,
        next_message_cursor,
    })
}

/// Reconstruct a thread's context as it stood at event-log id `as_of` (Cluster
/// 326). The message set (and each message's body) is folded from the **immutable
/// event log** — `MessagePosted`/`MessageEdited` carry the full `Message`,
/// `MessageTombstoned` its id — so a since-edited or since-tombstoned message
/// shows its as-of body, not its current one. The additive components (edits,
/// references, transitions, artifacts) are immutable rows cut by the anchor
/// event's time. Deterministic; no fresh semantic search. The glossary (current
/// vocabulary, not thread history) is omitted from an as-of pack.
async fn build_thread_context_as_of(
    store: &dyn Store,
    thread_id: ThreadId,
    as_of: i64,
    limits: ThreadContextLimits,
) -> Result<ThreadContext, ApiError> {
    // The anchor event bounds the replay (its id) and gives the time cutoff for
    // the additive components. A missing id is a client error (404 via NotFound).
    let anchor = store.get_stored_event(as_of).await?;
    let cutoff = anchor.occurred_at;

    let mut thread = store.get_thread(thread_id).await?;
    let channel = store.get_channel(thread.channel_id).await?;
    let workspace_id = channel.workspace_id;

    // Fold the thread's message events up to `as_of` into the as-of message set
    // (shared with the MCP builder via `maidan_types::reconstruct_messages_through`).
    let events = store.list_thread_events_through(thread_id, as_of).await?;
    let mut all_messages: Vec<Message> = reconstruct_messages_through(&events);

    // Keyset pagination over the reconstructed list (in memory — it is bounded).
    if let Some(cursor) = limits.message_cursor {
        match all_messages.iter().position(|m| m.id == cursor) {
            Some(pos) => all_messages = all_messages.split_off(pos + 1),
            None => all_messages.clear(),
        }
    }
    let page_limit = limits.message_limit.clamp(1, 500);
    let next_message_cursor = if all_messages.len() as i64 > page_limit {
        all_messages
            .get(page_limit as usize - 1)
            .map(|m| m.id.0.to_string())
    } else {
        None
    };
    let messages: Vec<Message> = all_messages.into_iter().take(page_limit as usize).collect();

    // Edits — immutable rows, cut by the anchor's time; ordered like the live pack.
    let message_ids: Vec<MessageId> = messages.iter().map(|m| m.id).collect();
    let message_pos: HashMap<MessageId, usize> = message_ids
        .iter()
        .enumerate()
        .map(|(i, id)| (*id, i))
        .collect();
    let mut message_edits = store
        .list_message_edits_for_messages(&message_ids, 20)
        .await?;
    message_edits.retain(|e| e.edited_at <= cutoff);
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

    // References — additive, cut by time.
    let message_src_ids: Vec<uuid::Uuid> = messages.iter().map(|m| m.id.0).collect();
    let mut references = store
        .list_references_from(RefSide::Thread, thread_id.0)
        .await?;
    let mut from_messages = store
        .list_references_from_many(RefSide::Message, &message_src_ids)
        .await?;
    references.append(&mut from_messages);
    references.retain(|r| r.created_at <= cutoff);
    references.sort_by_key(|r| r.created_at);
    references.dedup_by_key(|r| r.id);

    // Transitions — additive, cut by time; the as-of FSM state is the last one.
    let mut transitions = store
        .list_thread_transitions(thread_id, limits.transition_limit)
        .await?;
    transitions.retain(|t| t.occurred_at <= cutoff);
    let mut chrono = transitions.clone();
    chrono.sort_by_key(|t| t.occurred_at);
    let state = chrono
        .last()
        .map(|t| t.to_state)
        .unwrap_or(ThreadState::Open);
    thread.state = state;

    // Artifacts — from the as-of messages' metadata, cut by time.
    let mut artifact_shas = HashSet::new();
    for message in &messages {
        for sha in artifact_shas_from_metadata(&message.metadata) {
            artifact_shas.insert(sha);
        }
    }
    let mut artifacts = Vec::new();
    for sha in artifact_shas {
        if let Ok(artifact) = store.get_artifact_by_sha(&sha).await {
            if artifact.tombstoned_at.is_none() && artifact.created_at <= cutoff {
                artifacts.push(artifact);
            }
        }
    }
    artifacts.sort_by_key(|a| a.created_at);

    Ok(ThreadContext {
        workspace_id,
        channel_id: thread.channel_id,
        thread,
        messages,
        message_edits,
        references,
        artifacts,
        fsm: ThreadFsmContext { state, transitions },
        // An as-of pack omits the glossary (current vocabulary, not thread history).
        glossary: Vec::new(),
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
    // The glossary rides the workspace level once; the nested thread builds skip
    // it (below) so it is not repeated per thread.
    let glossary = if limits.include_glossary {
        store.list_glossary_terms(workspace_id).await?
    } else {
        Vec::new()
    };
    let nested_limits = ThreadContextLimits {
        include_glossary: false,
        ..limits
    };
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
    // Build each thread's context concurrently (bounded), preserving page order
    // (Cluster 199). Each build is ~7 independent store round-trips; a page of up
    // to 50 threads built sequentially stacked that latency linearly. `buffered`
    // keeps the output in page order and short-circuits on the first error, so
    // the response contract (and the tombstone-mid-build 404) is unchanged.
    let thread_ids: Vec<ThreadId> = page.iter().map(|t| t.id).collect();
    let threads: Vec<ThreadContext> = stream::iter(thread_ids)
        .map(|tid| build_thread_context(store, tid, nested_limits))
        .buffered(CONTEXT_THREAD_CONCURRENCY)
        .try_collect()
        .await?;
    Ok(WorkspaceContext {
        workspace,
        channels,
        threads,
        glossary,
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

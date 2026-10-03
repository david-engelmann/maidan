//! Read a thread or workspace context pack out of the store. Shared by REST
//! (`GET /threads/:id/context`) and MCP (`get_thread_context`) so both hand an
//! agent the same bytes for the same state, and a snapshot of either has the
//! same sha256. This module fetches the rows; the layout, ordering and
//! serialization are `maidan_types::assemble_thread_context`'s.

use futures::stream::{self, StreamExt, TryStreamExt};
use maidan_types::*;

use crate::{Store, StoreError};

/// Max concurrent per-thread builds inside a workspace pack. Each build is
/// about ten store round-trips; bounding the fan-out keeps one request from
/// saturating the connection pool while still overlapping a page's latency.
const CONTEXT_THREAD_CONCURRENCY: usize = 8;

/// Edits read per message, newest last.
const EDITS_PER_MESSAGE: i64 = 20;

#[derive(Debug, Clone, Copy)]
pub struct ThreadContextLimits {
    /// Messages per page, clamped to `1..=500`.
    pub message_limit: i64,
    /// Transitions listed, oldest first, clamped to `1..=200`.
    pub transition_limit: i64,
    pub message_cursor: Option<MessageId>,
    /// Include full `body_before`/`body_after` on each edit. Default `false`:
    /// edits carry metadata only. The single biggest token lever on a pack.
    pub include_edits: bool,
    /// Attach the workspace glossary. Default `true`. A workspace build turns
    /// it off on its nested threads so the glossary rides the top level once.
    pub include_glossary: bool,
    /// As-of replay: rebuild the thread as it stood at this event-log id,
    /// from the immutable log. `None` = the live pack.
    pub as_of: Option<i64>,
    /// Token budget for the message page; see
    /// [`maidan_types::fold_messages_to_budget`]. `None` = cap by rows only.
    /// Applies per thread, so a workspace pack budgets each nested thread.
    pub token_budget: Option<i64>,
    /// Attach parent grounding to a child thread's pack. Default `true`; off
    /// on a workspace pack's nested threads. Withheld for a cross-channel or
    /// DM parent (see [`maidan_types::ParentGrounding::assemble`]).
    pub include_parent_grounding: bool,
    /// Attach the channel's accepted decisions. Default `true`; off on a
    /// workspace pack's nested threads. Withheld for DM channels.
    pub include_accepted_decisions: bool,
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
            token_budget: None,
            include_parent_grounding: true,
            include_accepted_decisions: true,
        }
    }
}

/// A thread's context pack. A tombstoned thread is [`StoreError::NotFound`].
pub async fn build_thread_context(
    store: &dyn Store,
    thread_id: ThreadId,
    limits: ThreadContextLimits,
) -> Result<ThreadContext, StoreError> {
    match limits.as_of {
        Some(as_of) => build_as_of(store, thread_id, as_of, limits).await,
        None => build_live(store, thread_id, limits).await,
    }
}

async fn build_live(
    store: &dyn Store,
    thread_id: ThreadId,
    limits: ThreadContextLimits,
) -> Result<ThreadContext, StoreError> {
    let thread = store.get_thread(thread_id).await?;
    if thread.tombstoned_at.is_some() {
        return Err(StoreError::NotFound);
    }
    let channel = store.get_channel(thread.channel_id).await?;
    let workspace_id = channel.workspace_id;

    let page = store
        .list_messages_after(thread_id, limits.message_cursor, page_limit(limits) + 1)
        .await?;
    let (messages, next_message_cursor) = take_page(page, page_limit(limits));
    // Folded before the reads below, so references, edits and artifacts cover
    // only the kept messages and the whole pack shrinks.
    let (messages, elision) = apply_token_budget(messages, limits.token_budget);

    let transitions = store
        .list_thread_transitions(thread_id, transition_limit(limits))
        .await?;
    let references = collect_references(store, thread_id, &messages).await?;
    let edits = store
        .list_message_edits_for_messages(&message_ids(&messages), EDITS_PER_MESSAGE)
        .await?;
    let artifacts = collect_artifacts(store, workspace_id, &messages).await;
    let glossary = if limits.include_glossary {
        store.list_glossary_terms(workspace_id).await?
    } else {
        Vec::new()
    };
    let parent_grounding = if limits.include_parent_grounding {
        build_parent_grounding(store, &thread, &channel).await
    } else {
        None
    };
    let closed_results = if limits.include_accepted_decisions {
        channel_closed_results(store, &channel).await?
    } else {
        Vec::new()
    };
    let change_requests = store
        .list_reviews(thread_id)
        .await?
        .into_iter()
        .filter(|r| r.decision == ReviewDecision::RequestChanges)
        .collect();
    let required_skills = store
        .list_thread_required_skills(thread_id)
        .await?
        .into_iter()
        .map(|s| s.skill)
        .collect();

    Ok(assemble_thread_context(PackParts {
        workspace_id,
        thread,
        required_skills,
        glossary,
        parent_grounding,
        closed_results,
        messages,
        elision,
        edits,
        include_edit_bodies: limits.include_edits,
        references,
        artifacts,
        transitions,
        change_requests,
        as_of: None,
        next_message_cursor,
    }))
}

/// The thread as it stood at event-log id `as_of`, built only from what the
/// log and the immutable rows recorded by then, so nothing written after the
/// anchor changes its bytes. The message set and bodies are folded from
/// `MessagePosted`/`MessageEdited`/`MessageTombstoned`; the thread row is the
/// last snapshot an event carried, with `updated_at` set to the time of the
/// thread's last event at or before the anchor; edits, references, transitions
/// and artifacts are cut at the anchor's time. The live layers (glossary,
/// parent grounding, accepted decisions, change requests, required skills)
/// are omitted: they describe now, not then. Erasure still applies: a
/// tombstoned artifact or a shredded message body is not brought back.
async fn build_as_of(
    store: &dyn Store,
    thread_id: ThreadId,
    as_of: i64,
    limits: ThreadContextLimits,
) -> Result<ThreadContext, StoreError> {
    let anchor = store.get_stored_event(as_of).await?;
    let cutoff = anchor.occurred_at;
    let live = store.get_thread(thread_id).await?;
    let channel = store.get_channel(live.channel_id).await?;
    let workspace_id = channel.workspace_id;

    let events = store.list_thread_events_through(thread_id, as_of).await?;
    let mut transitions = store
        .list_thread_transitions(thread_id, transition_limit(limits))
        .await?;
    transitions.retain(|t| t.occurred_at <= cutoff);
    let thread = thread_as_of(&live, &events, &transitions);

    let mut all = reconstruct_messages_through(&events);
    if let Some(cursor) = limits.message_cursor {
        match all.iter().position(|m| m.id == cursor) {
            Some(pos) => all = all.split_off(pos + 1),
            None => all.clear(),
        }
    }
    let (messages, next_message_cursor) = take_page(all, page_limit(limits));
    let (messages, elision) = apply_token_budget(messages, limits.token_budget);

    let mut edits = store
        .list_message_edits_for_messages(&message_ids(&messages), EDITS_PER_MESSAGE)
        .await?;
    edits.retain(|e| e.edited_at <= cutoff);
    let mut references = collect_references(store, thread_id, &messages).await?;
    references.retain(|r| r.created_at <= cutoff);
    let mut artifacts = collect_artifacts(store, workspace_id, &messages).await;
    artifacts.retain(|a| a.created_at <= cutoff);

    Ok(assemble_thread_context(PackParts {
        workspace_id,
        thread,
        required_skills: Vec::new(),
        glossary: Vec::new(),
        parent_grounding: None,
        closed_results: Vec::new(),
        messages,
        elision,
        edits,
        include_edit_bodies: limits.include_edits,
        references,
        artifacts,
        transitions,
        change_requests: Vec::new(),
        as_of: Some(as_of),
        next_message_cursor,
    }))
}

/// The thread row as of the anchor: the last snapshot its events carried, else
/// the live row's fixed fields with the state the transitions reached and no
/// claim. `updated_at` is the time of its last event at or before the anchor
/// (its creation time when it has none), never the live row's, which every
/// later post moves.
fn thread_as_of(live: &Thread, events: &[StoredEvent], transitions: &[ThreadTransition]) -> Thread {
    let mut thread = reconstruct_thread_through(events).unwrap_or_else(|| Thread {
        state: transitions
            .iter()
            .max_by_key(|t| (t.occurred_at, t.id))
            .map(|t| t.to_state)
            .unwrap_or(ThreadState::Open),
        assignee_id: None,
        assignment_expires_at: None,
        claim_lease_id: None,
        work_started_at: None,
        ..live.clone()
    });
    thread.updated_at = events
        .last()
        .map(|e| e.occurred_at)
        .unwrap_or(thread.created_at);
    thread
}

/// A workspace pack: the workspace, its channels, a keyset page of threads
/// (`created_at, id`), and the glossary once at the top. Nested thread packs
/// carry no glossary, parent grounding or accepted decisions. The caller
/// filters the threads the reader may not see.
pub async fn build_workspace_context(
    store: &dyn Store,
    workspace_id: WorkspaceId,
    thread_limit: i64,
    thread_cursor: Option<ThreadId>,
    limits: ThreadContextLimits,
) -> Result<WorkspaceContext, StoreError> {
    let workspace = store.get_workspace(workspace_id).await?;
    let channels = store.list_channels(workspace_id).await?;
    let mut glossary = if limits.include_glossary {
        store.list_glossary_terms(workspace_id).await?
    } else {
        Vec::new()
    };
    glossary.sort_by(|a, b| a.term.cmp(&b.term).then_with(|| a.id.cmp(&b.id)));
    let nested = ThreadContextLimits {
        include_glossary: false,
        include_parent_grounding: false,
        include_accepted_decisions: false,
        as_of: None,
        message_cursor: None,
        ..limits
    };
    let page_limit = thread_limit.clamp(1, 50);
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
    // `buffered` keeps page order and stops at the first error, so a thread
    // tombstoned mid-build fails the request as it did when built in sequence.
    let threads = stream::iter(page.into_iter().map(|t| t.id))
        .map(|id| build_thread_context(store, id, nested))
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

fn page_limit(limits: ThreadContextLimits) -> i64 {
    limits.message_limit.clamp(1, 500)
}

fn transition_limit(limits: ThreadContextLimits) -> i64 {
    limits.transition_limit.clamp(1, 200)
}

/// The first `limit` of `messages`, and the cursor to the next page when there
/// are more.
fn take_page(messages: Vec<Message>, limit: i64) -> (Vec<Message>, Option<String>) {
    let limit = limit as usize;
    let next = (messages.len() > limit)
        .then(|| messages.get(limit - 1).map(|m| m.id.0.to_string()))
        .flatten();
    (messages.into_iter().take(limit).collect(), next)
}

fn apply_token_budget(
    messages: Vec<Message>,
    token_budget: Option<i64>,
) -> (Vec<Message>, Option<PackElision>) {
    match token_budget {
        Some(budget) => fold_messages_to_budget(messages, budget.max(1) as usize),
        None => (messages, None),
    }
}

fn message_ids(messages: &[Message]) -> Vec<MessageId> {
    messages.iter().map(|m| m.id).collect()
}

/// References from the thread and from each paged message: one read for the
/// thread and one batched read across the messages.
async fn collect_references(
    store: &dyn Store,
    thread_id: ThreadId,
    messages: &[Message],
) -> Result<Vec<Reference>, StoreError> {
    let src_ids: Vec<uuid::Uuid> = messages.iter().map(|m| m.id.0).collect();
    let mut references = store
        .list_references_from(RefSide::Thread, thread_id.0)
        .await?;
    references.extend(
        store
            .list_references_from_many(RefSide::Message, &src_ids)
            .await?,
    );
    Ok(references)
}

/// The artifacts the messages name, as the workspace sees them. A sha the
/// workspace holds no ref to, or a tombstoned one, is skipped: naming a sha in
/// a message's metadata is not access to it.
async fn collect_artifacts(
    store: &dyn Store,
    workspace_id: WorkspaceId,
    messages: &[Message],
) -> Vec<Artifact> {
    let mut artifacts = Vec::new();
    for sha in artifact_reference_order(messages) {
        if let Ok(artifact) = store.get_artifact_for_workspace(workspace_id, &sha).await {
            if artifact.tombstoned_at.is_none() {
                artifacts.push(artifact);
            }
        }
    }
    artifacts
}

/// The parent's opening message and latest result for a child thread, when
/// [`ParentGrounding::assemble`] allows it. Cross-channel and DM parents are
/// refused before the extra reads.
async fn build_parent_grounding(
    store: &dyn Store,
    thread: &Thread,
    channel: &Channel,
) -> Option<ParentGrounding> {
    let parent_id = thread.parent_thread_id?;
    if channel.name == DM_CHANNEL_NAME {
        return None;
    }
    let parent = store.get_thread(parent_id).await.ok()?;
    if parent.channel_id != channel.id || parent.tombstoned_at.is_some() {
        return None;
    }
    let opening_message = store
        .list_messages_after(parent_id, None, 1)
        .await
        .ok()
        .and_then(|mut v| v.drain(..).next());
    let latest_result = store
        .get_thread_result(parent_id)
        .await
        .ok()
        .flatten()
        .map(|r| r.result);
    ParentGrounding::assemble(
        parent,
        channel.id,
        channel.name == DM_CHANNEL_NAME,
        opening_message,
        latest_result,
    )
}

/// The channel's latest closed results. None on a DM channel: `__dm__` holds
/// unrelated conversations, so same channel is not same audience. Every thread
/// in a channel gets the same list, its own result included once it has one,
/// so the layer is the same bytes in each of their packs.
async fn channel_closed_results(
    store: &dyn Store,
    channel: &Channel,
) -> Result<Vec<ChannelClosedResult>, StoreError> {
    if channel.name == DM_CHANNEL_NAME {
        return Ok(Vec::new());
    }
    store
        .list_channel_closed_results(channel.id, None, ACCEPTED_DECISIONS_LIMIT)
        .await
}

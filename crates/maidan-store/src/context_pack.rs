//! Read a thread, channel boot, or workspace context pack out of the store.
//! REST and MCP both call this, then serialize the same types, so the same
//! state is the same bytes and one snapshot sha.

use futures::stream::{self, StreamExt, TryStreamExt};
use maidan_types::*;

use crate::{Store, StoreError};

/// Max concurrent per-thread builds inside a workspace pack.
const CONTEXT_THREAD_CONCURRENCY: usize = 8;

/// Edits read per message.
const EDITS_PER_MESSAGE: i64 = 20;

#[derive(Debug, Clone, Copy)]
pub struct ThreadContextLimits {
    /// Messages per page, clamped to `1..=500`.
    pub message_limit: i64,
    /// Transitions listed, oldest first, clamped to `1..=200`.
    pub transition_limit: i64,
    pub message_cursor: Option<MessageId>,
    /// Include full `body_before`/`body_after` on each edit. Default false.
    pub include_edits: bool,
    /// Attach the workspace glossary. A workspace build turns this off on
    /// nested threads so the glossary rides the top level once.
    pub include_glossary: bool,
    /// Rebuild the thread as it stood at this event-log id. `None` is live.
    pub as_of: Option<i64>,
    /// Token budget for the message page. `None` caps by rows only.
    pub token_budget: Option<i64>,
    /// Caller byte cap for the canonical pack. `None` means no cap. Elision
    /// grows by [`ELISION_BLOCK_MESSAGES`] until the pack fits or only the
    /// opening message and the newest remain.
    pub max_bytes: Option<i64>,
    pub include_parent_grounding: bool,
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
            max_bytes: None,
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

/// The boot pack for a channel: workspace id, channel id, the glossary and
/// the channel's accepted decisions, oldest first. The same bytes for every
/// caller who can read the channel. A thread pack's prefix starts with it.
pub async fn build_channel_boot(
    store: &dyn Store,
    channel_id: ChannelId,
) -> Result<BootPack, StoreError> {
    let channel = store.get_channel(channel_id).await?;
    let glossary = sorted_glossary(store, channel.workspace_id).await?;
    let closed = channel_closed_results(store, &channel).await?;
    Ok(BootPack {
        workspace_id: channel.workspace_id,
        channel_id: channel.id,
        glossary,
        accepted_decisions: assemble_accepted_decisions(closed),
    })
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
    let transitions = store
        .list_thread_transitions(thread_id, transition_limit(limits))
        .await?;
    let glossary = if limits.include_glossary {
        sorted_glossary(store, workspace_id).await?
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
    let change_requests: Vec<ThreadReview> = store
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

    fit_page(
        store,
        limits,
        FitInput {
            workspace_id,
            thread,
            required_skills,
            glossary,
            parent_grounding,
            closed_results,
            messages,
            edits_cutoff: None,
            include_edit_bodies: limits.include_edits,
            transitions,
            change_requests,
            as_of: None,
            next_message_cursor,
        },
    )
    .await
}

/// The thread as it stood at event-log id `as_of`. Messages come from the
/// log. The thread row is the last snapshot an event carried, with
/// `updated_at` set to the time of the thread's last event at or before the
/// anchor, never the live row. Edits, references, transitions and artifacts
/// are cut at the anchor's time. Glossary, grounding, decisions, change
/// requests and required skills describe now, so an as-of pack omits them.
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
    fit_page(
        store,
        limits,
        FitInput {
            workspace_id,
            thread,
            required_skills: Vec::new(),
            glossary: Vec::new(),
            parent_grounding: None,
            closed_results: Vec::new(),
            messages,
            edits_cutoff: Some(cutoff),
            include_edit_bodies: limits.include_edits,
            transitions,
            change_requests: Vec::new(),
            as_of: Some(as_of),
            next_message_cursor,
        },
    )
    .await
}

struct FitInput {
    workspace_id: WorkspaceId,
    thread: Thread,
    required_skills: Vec<String>,
    glossary: Vec<GlossaryTerm>,
    parent_grounding: Option<ParentGrounding>,
    closed_results: Vec<ChannelClosedResult>,
    messages: Vec<Message>,
    edits_cutoff: Option<chrono::DateTime<chrono::Utc>>,
    include_edit_bodies: bool,
    transitions: Vec<ThreadTransition>,
    change_requests: Vec<ThreadReview>,
    as_of: Option<i64>,
    next_message_cursor: Option<String>,
}

async fn fit_page(
    store: &dyn Store,
    limits: ThreadContextLimits,
    input: FitInput,
) -> Result<ThreadContext, StoreError> {
    let original = input.messages.clone();
    let (mut messages, mut elision) = apply_token_budget(original.clone(), limits.token_budget);
    let mut elided = elision
        .as_ref()
        .map(|e| e.elided_message_count)
        .unwrap_or(0);
    let cap = limits.max_bytes.map(|n| n.max(1) as usize);
    loop {
        let pack = assemble_kept(store, &input, &messages, elision.clone()).await?;
        let size = pack.canonical_bytes()?.len();
        let Some(cap) = cap else {
            return Ok(pack);
        };
        if size <= cap {
            return Ok(pack);
        }
        let next = (elided + ELISION_BLOCK_MESSAGES).min(original.len().saturating_sub(2));
        if next <= elided {
            return Ok(pack);
        }
        elided = next;
        let (kept, marker) = elide_middle(original.clone(), elided, &format!("a {cap}-byte cap"));
        messages = kept;
        elision = marker;
    }
}

async fn assemble_kept(
    store: &dyn Store,
    input: &FitInput,
    messages: &[Message],
    elision: Option<PackElision>,
) -> Result<ThreadContext, StoreError> {
    let mut references = collect_references(store, input.thread.id, messages).await?;
    if let Some(cutoff) = input.edits_cutoff {
        references.retain(|r| r.created_at <= cutoff);
    }
    let mut edits = store
        .list_message_edits_for_messages(&message_ids(messages), EDITS_PER_MESSAGE)
        .await?;
    if let Some(cutoff) = input.edits_cutoff {
        edits.retain(|e| e.edited_at <= cutoff);
    }
    let mut artifacts = collect_artifacts(store, input.workspace_id, messages).await;
    if let Some(cutoff) = input.edits_cutoff {
        artifacts.retain(|a| a.created_at <= cutoff);
    }
    assemble_thread_context(PackParts {
        workspace_id: input.workspace_id,
        thread: input.thread.clone(),
        required_skills: input.required_skills.clone(),
        glossary: input.glossary.clone(),
        parent_grounding: input.parent_grounding.clone(),
        closed_results: input.closed_results.clone(),
        messages: messages.to_vec(),
        elision,
        edits,
        include_edit_bodies: input.include_edit_bodies,
        references,
        artifacts,
        transitions: input.transitions.clone(),
        change_requests: input.change_requests.clone(),
        as_of: input.as_of,
        next_message_cursor: input.next_message_cursor.clone(),
    })
    .map_err(StoreError::from)
}

fn thread_as_of(live: &Thread, events: &[StoredEvent], transitions: &[ThreadTransition]) -> Thread {
    let mut thread = match reconstruct_thread_through(events) {
        Some(row) => row,
        None => Thread {
            state: transitions
                .iter()
                .max_by_key(|t| (t.occurred_at, t.id))
                .map(|t| t.to_state)
                .unwrap_or(ThreadState::Open),
            assignee_id: None,
            assignment_expires_at: None,
            claim_lease_id: None,
            work_started_at: None,
            updated_at: live.created_at,
            ..live.clone()
        },
    };
    if let Some(last) = events.last() {
        thread.updated_at = last.occurred_at;
    }
    thread
}

pub async fn build_workspace_context(
    store: &dyn Store,
    workspace_id: WorkspaceId,
    thread_limit: i64,
    thread_cursor: Option<ThreadId>,
    limits: ThreadContextLimits,
) -> Result<WorkspaceContext, StoreError> {
    let workspace = store.get_workspace(workspace_id).await?;
    let mut channels = store.list_channels(workspace_id).await?;
    channels.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.0.cmp(&b.id.0)));
    let glossary = if limits.include_glossary {
        sorted_glossary(store, workspace_id).await?
    } else {
        Vec::new()
    };
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

fn take_page(messages: Vec<Message>, limit: i64) -> (Vec<Message>, Option<String>) {
    let limit = limit as usize;
    let next = if messages.len() > limit {
        messages
            .get(limit.saturating_sub(1))
            .map(|m| m.id.0.to_string())
    } else {
        None
    };
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

async fn sorted_glossary(
    store: &dyn Store,
    workspace_id: WorkspaceId,
) -> Result<Vec<GlossaryTerm>, StoreError> {
    let mut glossary = store.list_glossary_terms(workspace_id).await?;
    glossary.sort_by(|a, b| a.term.cmp(&b.term).then_with(|| a.id.cmp(&b.id)));
    Ok(glossary)
}

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

async fn collect_artifacts(
    store: &dyn Store,
    workspace_id: WorkspaceId,
    messages: &[Message],
) -> Vec<Artifact> {
    let mut artifacts = Vec::new();
    for sha in artifact_reference_order(messages) {
        if let Ok(artifact) = store.get_artifact_for_workspace(workspace_id, &sha).await {
            artifacts.push(artifact);
        }
    }
    artifacts
}

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

/// The channel's latest closed results, including a thread's own once it has
/// one, so every thread in the channel shares the layer. None on a DM channel.
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

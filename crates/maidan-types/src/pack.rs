//! Thread context packs: the one shape REST and MCP both serve, and the
//! token-budget fold.
//!
//! Provider prompt caches (Anthropic, OpenAI, Gemini) match a byte-identical
//! prefix, and any changed byte invalidates everything after it. So a pack is
//! laid out stable-first: what a post, a claim or a renewal cannot change comes
//! first ([`ContextPrefix`]), most shared first, the messages grow at its end,
//! and what changes on every claim and post sits in a short tail
//! ([`ContextTail`]) after it. The same state always serializes to the same
//! bytes, on either surface.
//!
//! A pack caps message *rows* (`message_limit`), not *tokens*: a page of wide
//! `ContentBlock` messages within the row cap can still overflow a model's
//! context window, and a long middle is the region a model attends to least
//! ("Lost in the Middle"). [`fold_messages_to_budget`] caps a page by an
//! estimated token budget: it keeps the thread's framing (its first message)
//! and its most-recent tail, and records the elided middle in a
//! [`PackElision`]. It elides whole blocks of [`ELISION_BLOCK_MESSAGES`], so
//! most new messages only append to the kept set.
//!
//! The math is deliberately pure and model-independent (`chars/4`) so a caller
//! can budget a pack without a live tokenizer, and so the whole fold is
//! unit-testable with no store. `maidan_store::context_pack` reads the inputs
//! and [`assemble_thread_context`] orders and lays them out.

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::{ChannelId, ClaimLeaseId, MemberId, MessageId, ThreadId, WorkspaceId};
use crate::models::{
    artifact_shas_from_metadata, Artifact, Channel, ChannelClosedResult, GlossaryTerm, Message,
    MessageEdit, Reference, Thread, ThreadState, ThreadTransition, Workspace,
};
use crate::review::ThreadReview;
use crate::waiter::parse_waiter_result;

/// Rough prompt-token estimate: ~4 characters per token, the widely-cited
/// approximation for English BPE tokenizers. Exact counts are tokenizer-specific;
/// this is intentionally model-independent so the pack's budget math is cheap and
/// stable. `chars/4`, rounded up.
pub fn estimate_tokens(s: &str) -> usize {
    s.chars().count().div_ceil(4)
}

/// Estimated token cost of one message as it rides the pack: the serialized JSON
/// (what the agent actually receives), estimated at `chars/4`. A serialization
/// failure — not expected for a well-formed [`Message`] — falls back to the body
/// length so the budgeter never panics.
pub fn message_tokens(message: &Message) -> usize {
    match serde_json::to_string(message) {
        Ok(json) => estimate_tokens(&json),
        Err(_) => estimate_tokens(&message.body),
    }
}

/// An auditable record of the messages a context pack dropped to fit its token
/// budget. The pack keeps the thread's opening message (framing) and its
/// most-recent tail; the elided middle is summarized here, so the omission is
/// visible in the response and recoverable (page from `first_elided_id`, or
/// refetch the thread without a budget).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PackElision {
    /// Number of messages folded away.
    pub elided_message_count: usize,
    /// Estimated tokens the elided messages would have cost.
    pub elided_token_estimate: usize,
    /// The oldest elided message — page from here to recover the middle.
    pub first_elided_id: MessageId,
    /// The newest elided message.
    pub last_elided_id: MessageId,
    /// A human/agent-readable summary naming the omission and how to recover it.
    pub summary: String,
}

/// Grounding for a **child** thread's context pack: a compact orientation to
/// the parent it was spawned from, so a fresh claimer of a sub-task knows *why
/// it exists* (the parent's opening ask) and *what the parent concluded* (its
/// latest recorded decision). Deliberately bounded — one framing message plus
/// one decision payload, not the parent's whole history.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ParentGrounding {
    pub thread_id: ThreadId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub state: ThreadState,
    /// The parent's opening (oldest) message — the framing / task statement.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub opening_message: Option<Message>,
    /// The parent's latest recorded decision/result payload, if any.
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub latest_result: Option<serde_json::Value>,
}

impl ParentGrounding {
    /// Decide whether a child thread may carry grounding for its (already-fetched)
    /// parent, and build it. Returns `None` — grounding withheld — when:
    /// - the parent is in a **different channel** than the child (the child's
    ///   access does not imply access to a parent elsewhere), or
    /// - the child's channel is a **DM** channel (grounding is a task concept; DM
    ///   threads are conversational and share the one `__dm__` channel across
    ///   unrelated conversations, so same-channel is not same-audience), or
    /// - the parent is **tombstoned**.
    ///
    /// The same-channel + non-DM rule is what makes grounding safe without a second
    /// access check: a caller that passed `ensure_thread_access` on the child, whose
    /// parent shares that non-DM channel, is by construction allowed to read the
    /// parent. `child_channel_is_dm` and `child_channel_id` describe the *child's*
    /// channel.
    pub fn assemble(
        parent: Thread,
        child_channel_id: ChannelId,
        child_channel_is_dm: bool,
        opening_message: Option<Message>,
        latest_result: Option<serde_json::Value>,
    ) -> Option<Self> {
        if parent.tombstoned_at.is_some()
            || parent.channel_id != child_channel_id
            || child_channel_is_dm
        {
            return None;
        }
        Some(Self {
            thread_id: parent.id,
            title: parent.title,
            state: parent.state,
            opening_message,
            latest_result,
        })
    }
}

/// Default cap on in-channel accepted decisions attached to a live claimer
/// pack. Small on purpose: this is orientation, not a dump of every historical
/// result. The store clamps `1..=50`; the pack stays tighter so a busy channel
/// does not blow the token budget.
pub const ACCEPTED_DECISIONS_LIMIT: i64 = 10;

/// UTF-8 byte budget for an accepted-decision `summary`. Matches the search
/// snippet fallback so teasers stay similarly sized across surfaces.
pub const ACCEPTED_DECISION_SUMMARY_BYTES: usize = 240;

/// A token-lean view of a closed/archived in-channel decision, for the next
/// `claim_next` claimer's context pack. Deliberately **not** the full
/// `ThreadResult` JSON — `rendered` / findings stay on `GET
/// /threads/:id/result`. `result_kind` is a **namespaced schema string** (e.g.
/// `example.review.result/1`), never a closed enum.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AcceptedDecision {
    pub thread_id: ThreadId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub state: ThreadState,
    pub produced_by: MemberId,
    pub produced_at: DateTime<Utc>,
    /// Namespaced producer schema (e.g. `example.review.result/1`). Absent on opaque
    /// JSON that does not carry a string `result_kind`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
}

impl AcceptedDecision {
    /// Project a store row into the pack view. Returns `None` when a recognized
    /// waiter envelope (`maidan.waiter.result/1`) is **not** `reviewed` — those
    /// are in-flight / failed producer states, not accepted decisions. Opaque
    /// JSON on a terminal thread is treated as accepted; a free-form string
    /// `result_kind` is copied through without requiring the waiter schema.
    pub fn from_closed_result(row: ChannelClosedResult) -> Option<Self> {
        if let Some(waiter) = parse_waiter_result(&row.result) {
            if !waiter.is_reviewed() {
                return None;
            }
            let summary = waiter
                .summary
                .filter(|s| !s.is_empty())
                .or(waiter.rendered.filter(|s| !s.is_empty()))
                .map(|s| utf8_excerpt(&s, ACCEPTED_DECISION_SUMMARY_BYTES));
            return Some(Self {
                thread_id: row.thread_id,
                title: row.title,
                state: row.state,
                produced_by: row.produced_by,
                produced_at: row.produced_at,
                result_kind: Some(waiter.result_kind),
                status: Some(waiter.status),
                summary,
            });
        }
        let result_kind = row
            .result
            .get("result_kind")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let status = row
            .result
            .get("status")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        Some(Self {
            thread_id: row.thread_id,
            title: row.title,
            state: row.state,
            produced_by: row.produced_by,
            produced_at: row.produced_at,
            result_kind,
            status,
            summary: excerpt_opaque(&row.result),
        })
    }
}

/// The pack's decision layer, oldest first, from the store's rows (newest
/// `produced_at` first, the order that lets `LIMIT` keep the latest). Oldest
/// first means a new decision lands at the end of the layer instead of
/// shifting every byte after the first. Rows the pack should not show
/// (non-reviewed waiter envelopes) are dropped.
pub fn assemble_accepted_decisions(
    rows: impl IntoIterator<Item = ChannelClosedResult>,
) -> Vec<AcceptedDecision> {
    let mut decisions: Vec<AcceptedDecision> = rows
        .into_iter()
        .filter_map(AcceptedDecision::from_closed_result)
        .collect();
    decisions.reverse();
    decisions
}

fn excerpt_opaque(result: &serde_json::Value) -> Option<String> {
    if let Some(s) = result
        .get("summary")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
    {
        return Some(utf8_excerpt(s, ACCEPTED_DECISION_SUMMARY_BYTES));
    }
    let compact = serde_json::to_string(result).ok()?;
    Some(utf8_excerpt(&compact, ACCEPTED_DECISION_SUMMARY_BYTES))
}

/// Truncate to at most `max_bytes` on a char boundary; append an ellipsis when
/// anything was dropped. A teaser that splits a code point is worse than a
/// slightly shorter teaser.
fn utf8_excerpt(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    let mut out = s[..end].to_string();
    out.push('…');
    out
}

/// How many messages the token-budget fold elides at a time.
///
/// A fold that keeps exactly the newest messages that fit drops one more old
/// message for every new one, so the kept set, and every cached byte after the
/// opener, changes on every post. Eliding whole blocks instead means the kept
/// set only grows between steps, and a post changes the cached prefix only
/// when it tips the page into folding another block: about one post in 16.
/// The cost is slack: right after a step the page sits up to 16 messages under
/// its budget. At a few hundred tokens a message that is a few thousand tokens,
/// small next to the budgets a context window is worth setting; a smaller
/// block would spend more cache misses to save it, a larger one more slack.
pub const ELISION_BLOCK_MESSAGES: usize = 16;

/// Fold a page of messages (ordered oldest→newest) to fit `budget_tokens`,
/// preserving the thread's framing and recency ("Lost in the Middle"): the
/// first message and a suffix of the most-recent messages are kept verbatim,
/// and the elided middle is recorded in a [`PackElision`]. Returns the kept
/// messages (still oldest→newest) and the elision marker.
///
/// The middle goes in whole blocks of [`ELISION_BLOCK_MESSAGES`], counted from
/// the second message: the fold elides the fewest blocks that make the page
/// fit. Until another block has to go, a new message appends to the kept set
/// and leaves the marker as it was. A budget too small to hold what follows any
/// whole number of blocks keeps the newest messages that fit instead, and that
/// set changes with every message.
///
/// Returns `(messages, None)` — no fold — when the page already fits the budget,
/// or is too short to fold (0, 1, or 2 messages, where dropping the middle cannot
/// help without discarding the framing or the tail). The single newest message is
/// always kept even if it alone exceeds the budget: a pack with no recent message
/// is useless, and the honest elision marker still tells the reader the pack is
/// over budget.
pub fn fold_messages_to_budget(
    messages: Vec<Message>,
    budget_tokens: usize,
) -> (Vec<Message>, Option<PackElision>) {
    // With ≤2 messages there is no middle to fold: an over-budget 2-message page is
    // its framing plus its tail, and dropping either loses signal.
    if messages.len() <= 2 {
        return (messages, None);
    }
    let n = messages.len();
    let costs: Vec<usize> = messages.iter().map(message_tokens).collect();
    let mut suffix = vec![0usize; n + 1];
    for i in (0..n).rev() {
        suffix[i] = suffix[i + 1] + costs[i];
    }
    if suffix[0] <= budget_tokens {
        return (messages, None);
    }

    // Eliding `e` messages keeps the opener and messages[1 + e..].
    let fits = |e: usize| costs[0] + suffix[1 + e] <= budget_tokens;
    let max_elided = n - 2;
    let elided = (1usize..)
        .map(|blocks| blocks * ELISION_BLOCK_MESSAGES)
        .take_while(|&e| e <= max_elided)
        .find(|&e| fits(e))
        .unwrap_or_else(|| (1..=max_elided).find(|&e| fits(e)).unwrap_or(max_elided));

    let kept_tail_start = 1 + elided;
    let first_elided_id = messages[1].id;
    let last_elided_id = messages[elided].id;
    let elided_token_estimate: usize = costs[1..kept_tail_start].iter().sum();
    // Nothing here counts the kept messages: the marker rides every pack, and a
    // count that moved with each post would change it on every post.
    let summary = format!(
        "{elided} earlier messages (~{elided_token_estimate} tokens) are elided to fit a \
         {budget_tokens}-token budget; the opening message and every message after them are \
         kept. Page from message id {first_elided_id} (or refetch without token_budget) to \
         read them."
    );

    let kept: Vec<Message> = messages
        .into_iter()
        .enumerate()
        .filter(|(i, _)| *i == 0 || *i >= kept_tail_start)
        .map(|(_, message)| message)
        .collect();

    (
        kept,
        Some(PackElision {
            elided_message_count: elided,
            elided_token_estimate,
            first_elided_id,
            last_elided_id,
            summary,
        }),
    )
}

/// What a pack says about its thread that working the thread does not change:
/// a post, a claim or a renewal leaves it as it was, so it belongs in the
/// cached prefix. The fields those do change are in [`ThreadStatus`], in the
/// tail.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ThreadBrief {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_thread_id: Option<ThreadId>,
    /// The accountable party (see [`Thread::owner_id`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_id: Option<MemberId>,
    /// The skills a claimer must hold, sorted. Absent from an as-of pack: the
    /// event log does not record them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_skills: Vec<String>,
    pub created_at: DateTime<Utc>,
}

impl ThreadBrief {
    pub fn of(thread: &Thread, required_skills: Vec<String>) -> Self {
        Self {
            title: thread.title.clone(),
            parent_thread_id: thread.parent_thread_id,
            owner_id: thread.owner_id,
            required_skills,
            created_at: thread.created_at,
        }
    }
}

/// The fields of a thread that change as it is worked: its FSM state, its
/// claim and lease, and `updated_at`, which every post bumps. A pack carries
/// them in its tail, where changing them invalidates nothing cached.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ThreadStatus {
    pub state: ThreadState,
    /// Absent while the thread is unassigned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee_id: Option<MemberId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignment_expires_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claim_lease_id: Option<ClaimLeaseId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_started_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

impl ThreadStatus {
    pub fn of(thread: &Thread) -> Self {
        Self {
            state: thread.state,
            assignee_id: thread.assignee_id,
            assignment_expires_at: thread.assignment_expires_at,
            claim_lease_id: thread.claim_lease_id,
            work_started_at: thread.work_started_at,
            updated_at: thread.updated_at,
        }
    }
}

/// A context-pack edit record. The `body_before`/`body_after` diff copies are
/// the single largest token cost in a pack, so they are omitted unless the
/// caller asks for them (`include_edits=true`); the who/when/which-message
/// signal is always present.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MessageEditView {
    pub id: i64,
    pub message_id: MessageId,
    pub editor_id: MemberId,
    pub edited_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_before: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body_after: Option<String>,
}

impl MessageEditView {
    pub fn from_edit(edit: MessageEdit, include_bodies: bool) -> Self {
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

/// The stable part of a thread context pack, in serialization order, most
/// shared first: the workspace layer (its glossary), the channel layer (its
/// accepted decisions), the parent layer (grounding a child thread shares with
/// its siblings), then the thread's own layers (its brief, the messages and
/// their edits, references, artifacts, transitions, change requests). A post
/// appends to `messages` and changes nothing before the end of that layer.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ContextPrefix {
    pub workspace_id: WorkspaceId,
    /// The workspace glossary, sorted by term. Omitted when empty, when the
    /// caller opts out (`include_glossary=false`), from an as-of pack, and on a
    /// workspace pack's nested threads (it rides `WorkspaceContext.glossary`
    /// once instead).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub glossary: Vec<GlossaryTerm>,
    pub channel_id: ChannelId,
    /// The channel's latest accepted decisions, oldest first: token-lean
    /// teasers so a fresh claimer sees what the channel already decided. The
    /// same list for every thread in the channel. Omitted when empty, opted
    /// out, as-of, workspace-nested, or DM.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accepted_decisions: Vec<AcceptedDecision>,
    /// For a child thread: the parent's opening ask and latest decision. Only
    /// for a parent in the same non-DM channel, and only when requested
    /// (`include_parent_grounding`, default true); absent for root threads, as-of
    /// packs and workspace-nested packs.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_grounding: Option<ParentGrounding>,
    pub thread_id: ThreadId,
    pub thread: ThreadBrief,
    /// The page of messages, oldest first. Under a `token_budget` the middle
    /// may be elided (see `ContextTail::elision`); the first message is kept.
    pub messages: Vec<Message>,
    /// Edits of the paged messages, in message order then edit order.
    pub message_edits: Vec<MessageEditView>,
    /// References from the thread and its paged messages, oldest first.
    pub references: Vec<Reference>,
    /// Artifacts the paged messages reference, in the order they are first
    /// referenced.
    pub artifacts: Vec<Artifact>,
    /// FSM transitions, oldest first (`transition_limit`, default 50).
    pub transitions: Vec<ThreadTransition>,
    /// What a reviewer sent this thread back for: each `request_changes`
    /// review with its note, until that reviewer reviews again. Omitted when
    /// empty and in an as-of pack.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub change_requests: Vec<ThreadReview>,
}

/// The volatile tail of a pack, serialized after the prefix: what claims,
/// renewals and posts change.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ContextTail {
    pub thread_status: ThreadStatus,
    /// Set when a `token_budget` folded the message page. Absent when the page
    /// fit or no budget was given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elision: Option<PackElision>,
    /// The event-log id an as-of pack was rebuilt at.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub as_of: Option<i64>,
    /// Present when more messages exist (`message_id` cursor for the next page).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_message_cursor: Option<String>,
}

/// A thread context pack. It serializes as one JSON object: the
/// [`ContextPrefix`] fields, then the [`ContextTail`] fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ThreadContext {
    #[serde(flatten)]
    pub prefix: ContextPrefix,
    #[serde(flatten)]
    pub tail: ContextTail,
}

impl ThreadContext {
    /// The pack's canonical bytes: compact JSON, prefix fields then tail
    /// fields. REST serves these, MCP serves them as its text, and a snapshot
    /// stores them, so the same state has one sha256 everywhere.
    pub fn to_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }
}

/// A workspace context pack: the workspace, its channels, a page of thread
/// packs, and the glossary once.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WorkspaceContext {
    pub workspace: Workspace,
    pub channels: Vec<Channel>,
    pub threads: Vec<ThreadContext>,
    /// The workspace glossary, carried once here rather than on each nested
    /// thread pack. Omitted when empty or when `include_glossary=false`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub glossary: Vec<GlossaryTerm>,
    /// Present when more threads exist (`thread_id` cursor for the next page).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_thread_cursor: Option<String>,
}

/// The rows a thread pack is built from, in whatever order the store returned
/// them. [`assemble_thread_context`] orders and lays them out.
#[derive(Debug, Clone)]
pub struct PackParts {
    pub workspace_id: WorkspaceId,
    /// The thread as the pack shows it: the live row, or the row as of an
    /// event.
    pub thread: Thread,
    pub required_skills: Vec<String>,
    pub glossary: Vec<GlossaryTerm>,
    pub parent_grounding: Option<ParentGrounding>,
    /// In-channel closed results, as `Store::list_channel_closed_results`
    /// lists them.
    pub closed_results: Vec<ChannelClosedResult>,
    /// The kept page, oldest first.
    pub messages: Vec<Message>,
    pub elision: Option<PackElision>,
    pub edits: Vec<MessageEdit>,
    pub include_edit_bodies: bool,
    pub references: Vec<Reference>,
    pub artifacts: Vec<Artifact>,
    pub transitions: Vec<ThreadTransition>,
    pub change_requests: Vec<ThreadReview>,
    pub as_of: Option<i64>,
    pub next_message_cursor: Option<String>,
}

/// Lay out a thread pack. Every list is given a total order, a sort key that
/// ends in an id, so rows that tie on a timestamp come out the same on every
/// call and on either backend, whatever order the store returned them in.
pub fn assemble_thread_context(parts: PackParts) -> ThreadContext {
    let position: HashMap<MessageId, usize> = parts
        .messages
        .iter()
        .enumerate()
        .map(|(i, m)| (m.id, i))
        .collect();

    let mut edits = parts.edits;
    edits.sort_by_key(|e| {
        (
            position.get(&e.message_id).copied().unwrap_or(usize::MAX),
            e.edited_at,
            e.id,
        )
    });
    let message_edits = edits
        .into_iter()
        .map(|e| MessageEditView::from_edit(e, parts.include_edit_bodies))
        .collect();

    let mut references = parts.references;
    references.sort_by_key(|r| (r.created_at, r.id));
    references.dedup_by_key(|r| r.id);

    // First-reference order: a post that names a new artifact appends it.
    let first_named: HashMap<String, usize> = artifact_reference_order(&parts.messages)
        .into_iter()
        .enumerate()
        .map(|(i, sha)| (sha, i))
        .collect();
    let mut artifacts = parts.artifacts;
    artifacts.sort_by(|a, b| {
        let rank = |x: &Artifact| first_named.get(&x.sha256).copied().unwrap_or(usize::MAX);
        rank(a).cmp(&rank(b)).then_with(|| a.sha256.cmp(&b.sha256))
    });
    artifacts.dedup_by(|a, b| a.sha256 == b.sha256);

    let mut transitions = parts.transitions;
    transitions.sort_by_key(|t| (t.occurred_at, t.id));

    let mut change_requests = parts.change_requests;
    change_requests.sort_by_key(|r| (r.created_at, r.reviewer_id.0));

    // Byte order, not the database's collation, so both backends agree.
    let mut glossary = parts.glossary;
    glossary.sort_by(|a, b| a.term.cmp(&b.term).then_with(|| a.id.cmp(&b.id)));

    let mut accepted_decisions = assemble_accepted_decisions(parts.closed_results);
    accepted_decisions.sort_by_key(|d| (d.produced_at, d.thread_id.0));

    let mut required_skills = parts.required_skills;
    required_skills.sort();
    required_skills.dedup();

    ThreadContext {
        prefix: ContextPrefix {
            workspace_id: parts.workspace_id,
            glossary,
            channel_id: parts.thread.channel_id,
            accepted_decisions,
            parent_grounding: parts.parent_grounding,
            thread_id: parts.thread.id,
            thread: ThreadBrief::of(&parts.thread, required_skills),
            messages: parts.messages,
            message_edits,
            references,
            artifacts,
            transitions,
            change_requests,
        },
        tail: ContextTail {
            thread_status: ThreadStatus::of(&parts.thread),
            elision: parts.elision,
            as_of: parts.as_of,
            next_message_cursor: parts.next_message_cursor,
        },
    }
}

/// The artifact shas `messages` name in their metadata, each once, in the
/// order the messages first name them.
pub fn artifact_reference_order(messages: &[Message]) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut order = Vec::new();
    for message in messages {
        for sha in artifact_shas_from_metadata(&message.metadata) {
            if seen.insert(sha.clone()) {
                order.push(sha);
            }
        }
    }
    order
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{MemberId, ThreadId};
    use chrono::Utc;

    fn parent_thread(channel_id: ChannelId, tombstoned: bool) -> Thread {
        Thread {
            id: ThreadId::new(),
            channel_id,
            parent_thread_id: None,
            title: Some("parent task".into()),
            state: ThreadState::Open,
            assignee_id: None,
            assignment_expires_at: None,
            claim_lease_id: None,
            work_started_at: None,
            owner_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            tombstoned_at: tombstoned.then(Utc::now),
        }
    }

    #[test]
    fn grounding_is_produced_for_a_same_channel_non_dm_parent() {
        let ch = ChannelId::new();
        let g = ParentGrounding::assemble(
            parent_thread(ch, false),
            ch,
            false,
            Some(msg("do the thing")),
            Some(serde_json::json!({"decision": "ship"})),
        );
        let g = g.expect("same-channel non-dm parent grounds");
        assert_eq!(g.title.as_deref(), Some("parent task"));
        assert!(g.opening_message.is_some());
        assert_eq!(
            g.latest_result,
            Some(serde_json::json!({"decision": "ship"}))
        );
    }

    #[test]
    fn grounding_is_withheld_cross_channel_dm_or_tombstoned() {
        let ch = ChannelId::new();
        let other = ChannelId::new();
        // Cross-channel parent: the child's access does not imply the parent's.
        assert!(
            ParentGrounding::assemble(parent_thread(other, false), ch, false, None, None).is_none()
        );
        // DM channel: same-channel is not same-audience.
        assert!(
            ParentGrounding::assemble(parent_thread(ch, false), ch, true, None, None).is_none()
        );
        // Tombstoned parent.
        assert!(
            ParentGrounding::assemble(parent_thread(ch, true), ch, false, None, None).is_none()
        );
    }

    fn closed_row(result: serde_json::Value) -> ChannelClosedResult {
        ChannelClosedResult {
            thread_id: ThreadId::new(),
            title: Some("decided".into()),
            state: ThreadState::Closed,
            result,
            produced_by: MemberId::new(),
            produced_at: Utc::now(),
        }
    }

    #[test]
    fn reviewed_waiter_envelope_is_an_accepted_decision_with_namespaced_kind() {
        let row = closed_row(serde_json::json!({
            "schema": crate::WAITER_RESULT_SCHEMA,
            "result_kind": "example.review.result/1",
            "status": crate::STATUS_REVIEWED,
            "summary": "ship it",
            "rendered": "# huge body that must not ride the pack",
        }));
        let d = AcceptedDecision::from_closed_result(row).expect("reviewed waiter");
        assert_eq!(d.result_kind.as_deref(), Some("example.review.result/1"));
        assert_eq!(d.status.as_deref(), Some(crate::STATUS_REVIEWED));
        assert_eq!(d.summary.as_deref(), Some("ship it"));
        let json = serde_json::to_value(&d).unwrap();
        assert!(json.get("result").is_none());
        assert!(json.get("rendered").is_none());
        assert!(json.get("schema").is_none());
    }

    #[test]
    fn non_reviewed_waiter_envelope_is_dropped() {
        let row = closed_row(serde_json::json!({
            "schema": crate::WAITER_RESULT_SCHEMA,
            "result_kind": "example.review.result/1",
            "status": "failed",
            "summary": "tests red",
        }));
        assert!(AcceptedDecision::from_closed_result(row).is_none());
    }

    #[test]
    fn opaque_closed_result_is_accepted_without_a_kind_enum() {
        let row = closed_row(serde_json::json!({"decision": "use postgres"}));
        let d = AcceptedDecision::from_closed_result(row).expect("opaque");
        assert!(d.result_kind.is_none());
        assert!(d.status.is_none());
        assert!(d.summary.as_deref().is_some_and(|s| s.contains("postgres")));
    }

    #[test]
    fn free_form_result_kind_is_copied_without_the_waiter_schema() {
        // Not an enum: a producer that never wraps `maidan.waiter.result/1` can still
        // name its kind as a namespaced string, and a new kind does not need a
        // Maidan code change.
        let row = closed_row(serde_json::json!({
            "result_kind": "example.novel.result/9",
            "status": "accepted",
            "summary": "go east",
        }));
        let d = AcceptedDecision::from_closed_result(row).expect("free-form kind");
        assert_eq!(d.result_kind.as_deref(), Some("example.novel.result/9"));
        assert_eq!(d.status.as_deref(), Some("accepted"));
        assert_eq!(d.summary.as_deref(), Some("go east"));
    }

    #[test]
    fn assemble_drops_non_reviewed_waiters_and_lists_oldest_first() {
        let reviewed = closed_row(serde_json::json!({
            "schema": crate::WAITER_RESULT_SCHEMA,
            "result_kind": "example.review.result/1",
            "status": crate::STATUS_REVIEWED,
            "summary": "ok",
        }));
        let failed = closed_row(serde_json::json!({
            "schema": crate::WAITER_RESULT_SCHEMA,
            "result_kind": "example.review.result/1",
            "status": "failed",
        }));
        let opaque = closed_row(serde_json::json!({"decision": "later"}));
        let reviewed_id = reviewed.thread_id;
        let opaque_id = opaque.thread_id;
        // The store lists newest first; the pack lists oldest first.
        let packed = assemble_accepted_decisions([reviewed, failed, opaque]);
        assert_eq!(packed.len(), 2);
        assert_eq!(packed[0].thread_id, opaque_id);
        assert_eq!(packed[1].thread_id, reviewed_id);
    }

    #[test]
    fn utf8_excerpt_does_not_split_a_code_point() {
        let s = "é".repeat(200);
        let out = utf8_excerpt(&s, 10);
        assert!(out.ends_with('…'));
        assert!(out.is_char_boundary(out.len() - '…'.len_utf8()));
        assert!(out.len() <= 10 + '…'.len_utf8());
    }

    fn msg(body: &str) -> Message {
        Message {
            id: MessageId::new(),
            thread_id: ThreadId::new(),
            author_id: MemberId::new(),
            body: body.to_string(),
            metadata: serde_json::json!({}),
            content: None,
            posted_at: Utc::now(),
            edited_at: None,
            tombstoned_at: None,
        }
    }

    #[test]
    fn estimate_tokens_is_chars_over_four_rounded_up() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(estimate_tokens("abcde"), 2);
        assert_eq!(estimate_tokens(&"x".repeat(400)), 100);
    }

    #[test]
    fn a_pack_within_budget_is_not_folded() {
        let messages = vec![msg("a"), msg("b"), msg("c"), msg("d")];
        let (kept, elision) = fold_messages_to_budget(messages.clone(), 100_000);
        assert_eq!(kept.len(), messages.len());
        assert!(elision.is_none());
    }

    #[test]
    fn short_pages_never_fold() {
        for len in 0..=2 {
            let messages: Vec<Message> = (0..len).map(|_| msg(&"x".repeat(1000))).collect();
            let (kept, elision) = fold_messages_to_budget(messages.clone(), 1);
            assert_eq!(kept.len(), messages.len());
            assert!(elision.is_none());
        }
    }

    #[test]
    fn folding_keeps_the_opener_and_the_recent_tail_and_records_the_middle() {
        // Five fat messages, a budget that fits only the opener + roughly two tail
        // messages. Bodies are distinct so we can assert identity.
        let messages = vec![
            msg(&"o".repeat(400)), // opener  (~ >100 tokens serialized)
            msg(&"1".repeat(400)),
            msg(&"2".repeat(400)),
            msg(&"3".repeat(400)),
            msg(&"4".repeat(400)), // newest tail
        ];
        let per = message_tokens(&messages[0]);
        // Room for the opener + two more messages, not all five.
        let budget = per * 3 + per / 2;
        let (kept, elision) = fold_messages_to_budget(messages.clone(), budget);

        let elision = elision.expect("over-budget page folds");
        // The opener is always first; the newest is always last.
        assert_eq!(kept.first().unwrap().id, messages[0].id);
        assert_eq!(kept.last().unwrap().id, messages[4].id);
        // Some middle was elided and the kept set shrank.
        assert!(kept.len() < messages.len());
        assert!(elision.elided_message_count >= 1);
        // The elided ids point strictly inside the original middle.
        assert_eq!(elision.first_elided_id, messages[1].id);
        assert!(elision.elided_token_estimate > 0);
        assert!(elision.summary.contains("elided"));
        // Kept messages stay in oldest→newest order and none of them is elided.
        let kept_ids: Vec<_> = kept.iter().map(|m| m.id).collect();
        assert!(kept_ids.windows(2).all(|w| {
            let a = messages.iter().position(|m| m.id == w[0]).unwrap();
            let b = messages.iter().position(|m| m.id == w[1]).unwrap();
            a < b
        }));
    }

    #[test]
    fn a_tiny_budget_still_keeps_opener_and_the_single_newest() {
        let messages = vec![
            msg(&"o".repeat(400)),
            msg(&"1".repeat(400)),
            msg(&"2".repeat(400)),
            msg(&"3".repeat(400)),
        ];
        let (kept, elision) = fold_messages_to_budget(messages.clone(), 1);
        // Never empty: framing + the newest survive even under an impossible budget.
        assert_eq!(kept.first().unwrap().id, messages[0].id);
        assert_eq!(kept.last().unwrap().id, messages[3].id);
        assert_eq!(kept.len(), 2);
        let elision = elision.expect("folded");
        assert_eq!(elision.elided_message_count, 2);
        assert_eq!(elision.first_elided_id, messages[1].id);
        assert_eq!(elision.last_elided_id, messages[2].id);
    }

    /// `n` messages whose serialized sizes are all equal.
    fn equal_messages(n: usize) -> Vec<Message> {
        (0..n)
            .map(|i| Message {
                posted_at: at(0),
                ..msg(&format!("{i:05} {}", "x".repeat(200)))
            })
            .collect()
    }

    #[test]
    fn the_fold_elides_whole_blocks_so_most_posts_only_append() {
        let thread = equal_messages(200);
        let budget = message_tokens(&thread[0]) * 40;
        let mut previous: Option<(Vec<MessageId>, PackElision)> = None;
        let mut steps = 0;
        for n in 3..=thread.len() {
            let (kept, elision) = fold_messages_to_budget(thread[..n].to_vec(), budget);
            let ids: Vec<MessageId> = kept.iter().map(|m| m.id).collect();
            let Some(elision) = elision else {
                assert_eq!(kept.len(), n, "an unfolded page keeps everything");
                continue;
            };
            assert_eq!(
                elision.elided_message_count % ELISION_BLOCK_MESSAGES,
                0,
                "n={n}: a budget that holds a block elides whole blocks"
            );
            if let Some((before, previous_elision)) = &previous {
                if previous_elision.elided_message_count == elision.elided_message_count {
                    assert_eq!(&ids[..ids.len() - 1], &before[..], "n={n}: the post only appended");
                    assert_eq!(previous_elision.summary, elision.summary, "n={n}");
                } else {
                    steps += 1;
                }
            }
            previous = Some((ids, elision));
        }
        // 160 posts past the first fold, one step per 16 of them.
        assert_eq!(steps, (200 - 41) / ELISION_BLOCK_MESSAGES);
    }

    #[test]
    fn a_budget_under_one_block_keeps_the_newest_messages_that_fit() {
        let thread = equal_messages(30);
        let budget = message_tokens(&thread[0]) * 5;
        let (kept, elision) = fold_messages_to_budget(thread.clone(), budget);
        let elision = elision.expect("folded");
        assert_eq!(kept.len(), 5);
        assert_eq!(kept[0].id, thread[0].id);
        assert_eq!(kept[1].id, thread[26].id);
        assert_eq!(elision.elided_message_count, 25);
        assert_eq!(elision.last_elided_id, thread[25].id);
    }

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000 + secs, 0).unwrap()
    }

    fn id(n: u128) -> uuid::Uuid {
        uuid::Uuid::from_u128(n)
    }

    fn parts_with_ties() -> PackParts {
        let channel_id = ChannelId(id(2));
        let thread = Thread {
            id: ThreadId(id(3)),
            channel_id,
            parent_thread_id: None,
            title: Some("t".into()),
            state: ThreadState::Open,
            assignee_id: None,
            assignment_expires_at: None,
            claim_lease_id: None,
            work_started_at: None,
            owner_id: None,
            created_at: at(0),
            updated_at: at(9),
            tombstoned_at: None,
        };
        let member = MemberId(id(4));
        let thread_id = thread.id;
        let reference = |n| Reference {
            id: id(n),
            src_kind: crate::RefSide::Thread,
            src_id: thread_id.0,
            dst_kind: crate::RefSide::Thread,
            dst_id: id(99),
            relation: crate::RelationKind::from_wire("relates_to"),
            created_at: at(5),
        };
        let transition = |n| ThreadTransition {
            id: id(n),
            thread_id,
            from_state: ThreadState::Open,
            to_state: ThreadState::InReview,
            actor_id: member,
            occurred_at: at(5),
        };
        let review = |n| ThreadReview {
            thread_id,
            reviewer_id: MemberId(id(n)),
            decision: crate::ReviewDecision::RequestChanges,
            note: None,
            actor_id: None,
            created_at: at(5),
            updated_at: at(5),
            dismissed_at: None,
        };
        let closed = |n| ChannelClosedResult {
            thread_id: ThreadId(id(n)),
            title: None,
            state: ThreadState::Closed,
            result: serde_json::json!({"decision": n}),
            produced_by: member,
            produced_at: at(5),
        };
        let term = |n, term: &str| GlossaryTerm {
            id: id(n),
            workspace_id: WorkspaceId(id(1)),
            term: term.into(),
            definition: "d".into(),
            aliases: vec![],
            created_by: member,
            created_at: at(1),
            updated_at: at(1),
        };
        PackParts {
            workspace_id: WorkspaceId(id(1)),
            thread,
            required_skills: vec!["rust".into(), "review".into(), "rust".into()],
            glossary: vec![term(31, "b"), term(30, "B"), term(32, "a")],
            parent_grounding: None,
            closed_results: vec![closed(41), closed(43), closed(42)],
            messages: vec![],
            elision: None,
            edits: vec![],
            include_edit_bodies: false,
            references: vec![reference(12), reference(10), reference(11), reference(10)],
            artifacts: vec![],
            transitions: vec![transition(21), transition(20)],
            change_requests: vec![review(52), review(51)],
            as_of: None,
            next_message_cursor: None,
        }
    }

    #[test]
    fn assembly_gives_rows_that_tie_on_a_timestamp_their_id_order() {
        let pack = assemble_thread_context(parts_with_ties());
        let p = &pack.prefix;
        let refs: Vec<_> = p.references.iter().map(|r| r.id).collect();
        assert_eq!(refs, [id(10), id(11), id(12)], "sorted and deduped");
        let transitions: Vec<_> = p.transitions.iter().map(|t| t.id).collect();
        assert_eq!(transitions, [id(20), id(21)]);
        let reviewers: Vec<_> = p.change_requests.iter().map(|r| r.reviewer_id.0).collect();
        assert_eq!(reviewers, [id(51), id(52)]);
        let decisions: Vec<_> = p.accepted_decisions.iter().map(|d| d.thread_id.0).collect();
        assert_eq!(decisions, [id(41), id(42), id(43)]);
        let terms: Vec<_> = p.glossary.iter().map(|g| g.term.as_str()).collect();
        assert_eq!(terms, ["B", "a", "b"], "byte order, not a collation");
        assert_eq!(p.thread.required_skills, ["review", "rust"]);

        let mut shuffled = parts_with_ties();
        shuffled.references.reverse();
        shuffled.transitions.reverse();
        shuffled.change_requests.reverse();
        shuffled.closed_results.reverse();
        shuffled.glossary.reverse();
        assert_eq!(
            assemble_thread_context(shuffled).to_bytes().unwrap(),
            pack.to_bytes().unwrap(),
            "the store's row order does not reach the bytes"
        );
    }

    #[test]
    fn the_pack_serializes_its_layers_most_shared_first_and_the_volatile_tail_last() {
        let json = String::from_utf8(assemble_thread_context(parts_with_ties()).to_bytes().unwrap())
            .unwrap();
        let order = [
            "\"workspace_id\"",
            "\"glossary\"",
            "\"channel_id\"",
            "\"accepted_decisions\"",
            "\"thread_id\"",
            "\"thread\"",
            "\"messages\"",
            "\"message_edits\"",
            "\"references\"",
            "\"artifacts\"",
            "\"transitions\"",
            "\"change_requests\"",
            "\"thread_status\"",
        ];
        let at: Vec<usize> = order
            .iter()
            .map(|key| json.find(&format!("{key}:")).unwrap_or_else(|| panic!("{key} missing")))
            .collect();
        assert!(at.windows(2).all(|w| w[0] < w[1]), "{json}");
        // The brief carries nothing a post or a claim changes.
        let brief = serde_json::to_value(&ThreadBrief::of(&parts_with_ties().thread, vec![])).unwrap();
        for volatile in ["updated_at", "state", "assignee_id", "claim_lease_id"] {
            assert!(brief.get(volatile).is_none(), "{volatile} is in the brief");
        }
    }

    #[test]
    fn artifacts_are_ordered_by_the_message_that_first_names_them() {
        let mut first = msg("a");
        first.metadata = serde_json::json!({"artifacts": ["cc", "aa"]});
        let mut second = msg("b");
        second.metadata = serde_json::json!({"artifact_sha256": "bb", "artifacts": ["cc"]});
        assert_eq!(artifact_reference_order(&[first, second]), ["aa", "cc", "bb"]);
    }
}

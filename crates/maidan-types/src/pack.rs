//! The one thread context pack REST and MCP both serve, and the fold that
//! keeps it cache-stable.
//!
//! Provider prompt caches reuse a byte-identical prefix, and one changed byte
//! invalidates everything after it. A pack is therefore laid out stable-first:
//! the workspace boot (identity, glossary, accepted decisions) leads, so the
//! thread pack's prefix starts with that boot; the stable thread brief, parent
//! grounding, messages, references, artifacts, transitions and change requests
//! follow; the volatile tail (state, lease, `updated_at`, elision, cursors,
//! the prefix sha) is last. The same state serializes to the same bytes on
//! either surface.
//!
//! A pack also caps message rows. [`fold_messages_to_budget`] drops the middle
//! in blocks of [`ELISION_BLOCK_MESSAGES`], keeping the opening message and the
//! recent tail, so a new message appends and the elision boundary moves about
//! once a block rather than on every post. The estimate is `chars/4`, with no
//! tokenizer. `maidan_store::context_pack` reads the rows and
//! [`assemble_thread_context`] lays them out.

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

/// Oldest first. The store lists newest `produced_at` first so `LIMIT` keeps
/// the latest; the pack reverses that, and breaks a `produced_at` tie by
/// `thread_id`, so a new decision lands at the end and both backends agree.
pub fn assemble_accepted_decisions(
    rows: impl IntoIterator<Item = ChannelClosedResult>,
) -> Vec<AcceptedDecision> {
    let mut decisions: Vec<AcceptedDecision> = rows
        .into_iter()
        .filter_map(AcceptedDecision::from_closed_result)
        .collect();
    decisions.sort_by(|a, b| {
        a.produced_at
            .cmp(&b.produced_at)
            .then_with(|| a.thread_id.0.cmp(&b.thread_id.0))
    });
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
/// message for every new one, so the kept set changes on every post. Eliding
/// whole blocks instead means the kept set only grows between steps, and a
/// post moves the boundary about once every 16 messages. The cost is slack:
/// right after a step the page sits up to a block under its budget.
pub const ELISION_BLOCK_MESSAGES: usize = 16;

/// Drop `elided` messages of the middle (index 1 onward), keeping the opening
/// message and everything after the gap. `elided == 0` returns the page
/// unchanged. `budget_label` is named in the marker (`"a 400-token budget"`,
/// `"a 8000-byte cap"`).
pub fn elide_middle(
    messages: Vec<Message>,
    elided: usize,
    budget_label: &str,
) -> (Vec<Message>, Option<PackElision>) {
    if messages.len() <= 2 || elided == 0 {
        return (messages, None);
    }
    let elided = elided.min(messages.len() - 2);
    let costs: Vec<usize> = messages.iter().map(message_tokens).collect();
    let kept_tail_start = 1 + elided;
    let first_elided_id = messages[1].id;
    let last_elided_id = messages[elided].id;
    let elided_token_estimate: usize = costs[1..kept_tail_start].iter().sum();
    let summary = format!(
        "{elided} earlier messages (~{elided_token_estimate} tokens) are elided to fit \
         {budget_label}; the opening message and every message after them are kept. \
         Page from message id {first_elided_id} to read them."
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

/// Fold a page of messages (oldest to newest) to fit `budget_tokens`.
///
/// The first message and a suffix of the most recent messages are kept. The
/// middle goes in whole blocks of [`ELISION_BLOCK_MESSAGES`], the fewest blocks
/// that make the page fit, so a new message appends until another block has to
/// go. A page too small to hold a whole block, or a budget too small for any
/// whole number of blocks, drops the fewest individual messages that fit
/// instead. Returns the page unchanged when it already fits, or when it has
/// fewer than three messages (there is no middle). The newest message is kept
/// even when it alone exceeds the budget.
pub fn fold_messages_to_budget(
    messages: Vec<Message>,
    budget_tokens: usize,
) -> (Vec<Message>, Option<PackElision>) {
    if messages.len() <= 2 {
        return (messages, None);
    }
    let n = messages.len();
    let costs: Vec<usize> = messages.iter().map(message_tokens).collect();
    let total: usize = costs.iter().sum();
    if total <= budget_tokens {
        return (messages, None);
    }
    let mut suffix = vec![0usize; n + 1];
    for i in (0..n).rev() {
        suffix[i] = suffix[i + 1] + costs[i];
    }
    let fits = |e: usize| costs[0] + suffix[1 + e] <= budget_tokens;
    let max_elided = n - 2;
    let stepped = (1usize..)
        .map(|blocks| blocks * ELISION_BLOCK_MESSAGES)
        .take_while(|&e| e <= max_elided)
        .find(|&e| fits(e));
    let elided = match stepped {
        Some(e) => e,
        None => (1..=max_elided).find(|&e| fits(e)).unwrap_or(max_elided),
    };
    elide_middle(messages, elided, &format!("a {budget_tokens}-token budget"))
}

/// What working a thread does not change: a post, a claim or a renewal leaves
/// it as it was, so it belongs in the cached prefix. State and the lease are
/// in the tail.
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
    /// Skills a claimer must hold, sorted. Absent from an as-of pack: the
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

/// The workspace and channel layers every agent of that channel shares.
/// A thread pack's prefix starts with these bytes. Glossary and accepted
/// decisions are omitted when empty, on both this object and the thread
/// prefix, so the byte prefix still holds.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BootPack {
    pub workspace_id: WorkspaceId,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub glossary: Vec<GlossaryTerm>,
    pub channel_id: ChannelId,
    /// The channel's accepted decisions, oldest first. The same list for
    /// every thread in the channel.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accepted_decisions: Vec<AcceptedDecision>,
}

impl BootPack {
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }
}

/// The stable part of a thread context pack, in serialization order. The
/// leading fields are the [`BootPack`]. A post appends to `messages` and
/// changes nothing before that key.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ContextPrefix {
    pub workspace_id: WorkspaceId,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub glossary: Vec<GlossaryTerm>,
    pub channel_id: ChannelId,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub accepted_decisions: Vec<AcceptedDecision>,
    pub thread_id: ThreadId,
    pub thread: ThreadBrief,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_grounding: Option<ParentGrounding>,
    pub messages: Vec<Message>,
    pub message_edits: Vec<MessageEditView>,
    pub references: Vec<Reference>,
    pub artifacts: Vec<Artifact>,
    pub transitions: Vec<ThreadTransition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub change_requests: Vec<ThreadReview>,
}

impl ContextPrefix {
    pub fn boot(&self) -> BootPack {
        BootPack {
            workspace_id: self.workspace_id,
            channel_id: self.channel_id,
            glossary: self.glossary.clone(),
            accepted_decisions: self.accepted_decisions.clone(),
        }
    }

    /// The compact JSON of this prefix begins with the compact JSON of its
    /// boot, minus the boot object's closing brace.
    pub fn starts_with_boot(&self) -> Result<bool, serde_json::Error> {
        let prefix = serde_json::to_vec(self)?;
        let boot = self.boot().canonical_bytes()?;
        Ok(!boot.is_empty() && prefix.starts_with(&boot[..boot.len() - 1]))
    }
}

/// The volatile tail, serialized after the prefix.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ContextTail {
    pub state: ThreadState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee_id: Option<MemberId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignment_expires_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claim_lease_id: Option<ClaimLeaseId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_started_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub elision: Option<PackElision>,
    /// The event-log id an as-of pack was rebuilt at.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub as_of: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_message_cursor: Option<String>,
    /// SHA-256 of the prefix's compact JSON, hex.
    pub prefix_sha256: String,
    /// Length of that prefix JSON, in bytes.
    pub prefix_bytes: u64,
}

impl ContextTail {
    pub fn of(
        thread: &Thread,
        elision: Option<PackElision>,
        as_of: Option<i64>,
        next_message_cursor: Option<String>,
        prefix_sha256: String,
        prefix_bytes: u64,
    ) -> Self {
        Self {
            state: thread.state,
            assignee_id: thread.assignee_id,
            assignment_expires_at: thread.assignment_expires_at,
            claim_lease_id: thread.claim_lease_id,
            work_started_at: thread.work_started_at,
            updated_at: thread.updated_at,
            elision,
            as_of,
            next_message_cursor,
            prefix_sha256,
            prefix_bytes,
        }
    }
}

/// A context-pack edit record. The before/after bodies are omitted unless the
/// caller asks (`include_edits`); the who/when/which-message signal stays.
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

/// A thread context pack. It serializes as one JSON object: the prefix fields,
/// then the tail fields. REST serves these bytes. MCP serves the prefix and
/// the tail as two content parts. A snapshot stores these bytes, so the same
/// state has one sha256 everywhere.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ThreadContext {
    #[serde(flatten)]
    pub prefix: ContextPrefix,
    #[serde(flatten)]
    pub tail: ContextTail,
}

impl ThreadContext {
    pub fn thread_id(&self) -> ThreadId {
        self.prefix.thread_id
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }

    pub fn prefix_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(&self.prefix)
    }

    pub fn tail_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string(&self.tail)
    }

    /// Bytes of the canonical pack strictly before the `"messages"` key.
    pub fn bytes_before_messages(&self) -> Result<Vec<u8>, serde_json::Error> {
        let raw = self.canonical_bytes()?;
        let needle = b"\"messages\":";
        match raw.windows(needle.len()).position(|w| w == needle) {
            Some(pos) => Ok(raw[..pos].to_vec()),
            None => Ok(Vec::new()),
        }
    }
}

/// A workspace context pack. Nested thread packs carry no glossary, parent
/// grounding or accepted decisions; the glossary rides here once.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WorkspaceContext {
    pub workspace: Workspace,
    pub channels: Vec<Channel>,
    pub threads: Vec<ThreadContext>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub glossary: Vec<GlossaryTerm>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_thread_cursor: Option<String>,
}

/// What a caller asked for beyond the rows: a delta since a prefix sha or a
/// message cursor, instead of the whole pack.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ContextDelta {
    pub delta: bool,
    pub prefix_unchanged: bool,
    pub prefix_sha256: String,
    pub prefix_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub since_message_cursor: Option<String>,
    /// Present when a message cursor's suffix can be appended to the prefix the
    /// caller already holds and that result hashes to `prefix_sha256`: only
    /// the messages after that cursor.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub messages: Option<Vec<Message>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_edits: Option<Vec<MessageEditView>>,
    /// Present when the caller has to replace the prefix it held: the sha
    /// changed and there is no cursor, or the cursor's suffix would not
    /// rebuild a prefix that hashes to `prefix_sha256` (a glossary term, an
    /// accepted decision, a reference, an artifact, a transition, a change
    /// request, an earlier edit, or the elision boundary moved).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefix: Option<ContextPrefix>,
    pub tail: ContextTail,
}

impl ContextDelta {
    /// The stable part (everything but the tail) and the tail, as compact JSON.
    pub fn parts(&self) -> Result<(String, String), serde_json::Error> {
        let tail = serde_json::to_string(&self.tail)?;
        let head = DeltaHead {
            delta: self.delta,
            prefix_unchanged: self.prefix_unchanged,
            prefix_sha256: self.prefix_sha256.clone(),
            prefix_bytes: self.prefix_bytes,
            since_message_cursor: self.since_message_cursor.clone(),
            messages: self.messages.clone(),
            message_edits: self.message_edits.clone(),
            prefix: self.prefix.clone(),
        };
        Ok((serde_json::to_string(&head)?, tail))
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }
}

#[derive(Serialize)]
struct DeltaHead {
    delta: bool,
    prefix_unchanged: bool,
    prefix_sha256: String,
    prefix_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    since_message_cursor: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    messages: Option<Vec<Message>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    message_edits: Option<Vec<MessageEditView>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    prefix: Option<ContextPrefix>,
}

/// REST `split=true`: the two parts MCP returns, plus the prefix sha and length.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SplitContext {
    pub prefix_sha256: String,
    pub prefix_bytes: u64,
    /// Compact JSON of the stable part. MCP content part 1.
    pub prefix: String,
    /// Compact JSON of the volatile tail. MCP content part 2.
    pub tail: String,
}

impl SplitContext {
    pub fn from_parts(
        prefix_sha256: String,
        prefix_bytes: u64,
        prefix: String,
        tail: String,
    ) -> Self {
        Self {
            prefix_sha256,
            prefix_bytes,
            prefix,
            tail,
        }
    }

    pub fn canonical_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec(self)
    }
}

/// The rows a thread pack is built from, in whatever order the store returned
/// them. [`assemble_thread_context`] orders and lays them out.
#[derive(Debug, Clone)]
pub struct PackParts {
    pub workspace_id: WorkspaceId,
    /// The thread as the pack shows it: the live row, or the row as of an event.
    pub thread: Thread,
    pub required_skills: Vec<String>,
    pub glossary: Vec<GlossaryTerm>,
    pub parent_grounding: Option<ParentGrounding>,
    /// In-channel closed results, as the store lists them.
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

/// Lay out a thread pack. Every list gets a total order whose last key is an
/// id, so rows that tie on a timestamp come out the same on every call.
pub fn assemble_thread_context(parts: PackParts) -> Result<ThreadContext, serde_json::Error> {
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

    let mut glossary = parts.glossary;
    glossary.sort_by(|a, b| a.term.cmp(&b.term).then_with(|| a.id.cmp(&b.id)));

    let accepted_decisions = if parts.closed_results.is_empty() {
        Vec::new()
    } else {
        assemble_accepted_decisions(parts.closed_results)
    };

    let mut required_skills = parts.required_skills;
    required_skills.sort();
    required_skills.dedup();

    let prefix = ContextPrefix {
        workspace_id: parts.workspace_id,
        glossary,
        channel_id: parts.thread.channel_id,
        accepted_decisions,
        thread_id: parts.thread.id,
        thread: ThreadBrief::of(&parts.thread, required_skills),
        parent_grounding: parts.parent_grounding,
        messages: parts.messages,
        message_edits,
        references,
        artifacts,
        transitions,
        change_requests,
    };
    let prefix_raw = serde_json::to_vec(&prefix)?;
    let prefix_sha256 = sha256_hex(&prefix_raw);
    let prefix_bytes = prefix_raw.len() as u64;
    Ok(ThreadContext {
        tail: ContextTail::of(
            &parts.thread,
            parts.elision,
            parts.as_of,
            parts.next_message_cursor,
            prefix_sha256,
            prefix_bytes,
        ),
        prefix,
    })
}

/// Frame a built pack as a delta when the caller asked for one.
///
/// `full` is the pack with no message cursor (the prefix a cache would hold).
/// `paged` is that pack, or the page after `message_cursor` when one was set.
/// A `since_prefix_sha` that matches `full` returns only the tail. A cursor
/// returns the messages after the cursor when appending them rebuilds a prefix
/// that hashes to `prefix_sha256`, and when the caller named no prefix sha
/// (they asked for the slice, not a cache check). Otherwise the delta carries
/// the replacement prefix: splicing messages alone would not hash.
pub fn frame_delta(
    full: &ThreadContext,
    paged: &ThreadContext,
    since_prefix_sha: Option<&str>,
    message_cursor: Option<MessageId>,
) -> ContextDelta {
    let unchanged = since_prefix_sha.is_some_and(|sha| sha == full.tail.prefix_sha256);
    if unchanged {
        return ContextDelta {
            delta: true,
            prefix_unchanged: true,
            prefix_sha256: full.tail.prefix_sha256.clone(),
            prefix_bytes: full.tail.prefix_bytes,
            since_message_cursor: None,
            messages: None,
            message_edits: None,
            prefix: None,
            tail: full.tail.clone(),
        };
    }
    if let Some(cursor) = message_cursor {
        let appends =
            since_prefix_sha.is_none_or(|sha| cursor_suffix_rebuilds(full, paged, cursor, sha));
        if appends {
            return ContextDelta {
                delta: true,
                prefix_unchanged: false,
                prefix_sha256: full.tail.prefix_sha256.clone(),
                prefix_bytes: full.tail.prefix_bytes,
                since_message_cursor: Some(cursor.0.to_string()),
                messages: Some(paged.prefix.messages.clone()),
                message_edits: Some(paged.prefix.message_edits.clone()),
                prefix: None,
                tail: paged.tail.clone(),
            };
        }
    }
    ContextDelta {
        delta: true,
        prefix_unchanged: false,
        prefix_sha256: full.tail.prefix_sha256.clone(),
        prefix_bytes: full.tail.prefix_bytes,
        since_message_cursor: None,
        messages: None,
        message_edits: None,
        prefix: Some(full.prefix.clone()),
        tail: full.tail.clone(),
    }
}

/// True when `since_prefix_sha` is the prefix of `full` cut at `cursor`
/// (inclusive) and the messages after that cursor are exactly `paged`, with
/// no new reference or artifact the message slice cannot carry.
fn cursor_suffix_rebuilds(
    full: &ThreadContext,
    paged: &ThreadContext,
    cursor: MessageId,
    since_prefix_sha: &str,
) -> bool {
    let Some(pos) = full.prefix.messages.iter().position(|m| m.id == cursor) else {
        return false;
    };
    let mut through = full.prefix.clone();
    let kept: HashSet<MessageId> = through.messages[..=pos].iter().map(|m| m.id).collect();
    let kept_src: HashSet<uuid::Uuid> = kept.iter().map(|id| id.0).collect();
    through.messages.truncate(pos + 1);
    through
        .message_edits
        .retain(|edit| kept.contains(&edit.message_id));
    through.references.retain(|reference| {
        reference.src_kind != crate::models::RefSide::Message
            || kept_src.contains(&reference.src_id)
    });
    let named: HashSet<String> = artifact_reference_order(&through.messages)
        .into_iter()
        .collect();
    through
        .artifacts
        .retain(|artifact| named.contains(&artifact.sha256));
    let Ok(raw) = serde_json::to_vec(&through) else {
        return false;
    };
    if sha256_hex(&raw) != since_prefix_sha {
        return false;
    }
    // A new message that names a reference or an artifact changes bytes the
    // message slice does not carry. The caller would append and miss them.
    if through.references.len() != full.prefix.references.len()
        || through.artifacts.len() != full.prefix.artifacts.len()
    {
        return false;
    }
    let suffix = &full.prefix.messages[pos + 1..];
    let (Ok(suffix_bytes), Ok(paged_bytes)) = (
        serde_json::to_vec(suffix),
        serde_json::to_vec(&paged.prefix.messages),
    ) else {
        return false;
    };
    if suffix_bytes != paged_bytes {
        return false;
    }
    let suffix_ids: HashSet<MessageId> = suffix.iter().map(|m| m.id).collect();
    let suffix_edits: Vec<_> = full
        .prefix
        .message_edits
        .iter()
        .filter(|edit| suffix_ids.contains(&edit.message_id))
        .cloned()
        .collect();
    suffix_edits == paged.prefix.message_edits
}

/// The artifact shas `messages` name, each once, in first-reference order.
/// Within one message the extractor sorts by sha, so a tie is stable.
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

fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0xf) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::{MemberId, ThreadId};
    use chrono::{DateTime, Utc};

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
            block: None,
            closed_without_review: false,
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

    fn at_secs(secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000 + secs, 0).expect("fixed timestamp")
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
        let mut reviewed = closed_row(serde_json::json!({
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
        let mut opaque = closed_row(serde_json::json!({"decision": "later"}));
        let reviewed_id = reviewed.thread_id;
        let opaque_id = opaque.thread_id;
        // Newest first, as the store lists them. Opaque is older.
        reviewed.produced_at = at_secs(20);
        opaque.produced_at = at_secs(10);
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

    fn uid(n: u128) -> uuid::Uuid {
        uuid::Uuid::from_u128(n)
    }

    fn fixed_message(n: u128, body: &str) -> Message {
        Message {
            id: MessageId(uid(n)),
            thread_id: ThreadId(uid(1)),
            author_id: MemberId(uid(3)),
            body: body.to_string(),
            metadata: serde_json::json!({}),
            content: None,
            posted_at: at_secs(n as i64),
            edited_at: None,
            tombstoned_at: None,
        }
    }

    fn fixed_thread(updated: i64) -> Thread {
        Thread {
            id: ThreadId(uid(1)),
            channel_id: ChannelId(uid(2)),
            parent_thread_id: None,
            title: Some("golden".into()),
            state: ThreadState::Open,
            assignee_id: None,
            assignment_expires_at: None,
            claim_lease_id: None,
            work_started_at: None,
            owner_id: None,
            created_at: at_secs(1),
            updated_at: at_secs(updated),
            tombstoned_at: None,
            block: None,
            closed_without_review: false,
        }
    }

    fn pack_with(messages: Vec<Message>, updated: i64) -> ThreadContext {
        assemble_thread_context(PackParts {
            workspace_id: WorkspaceId(uid(4)),
            thread: fixed_thread(updated),
            required_skills: vec!["beta".into(), "alpha".into()],
            glossary: Vec::new(),
            parent_grounding: None,
            closed_results: Vec::new(),
            messages,
            elision: None,
            edits: Vec::new(),
            include_edit_bodies: false,
            references: Vec::new(),
            artifacts: Vec::new(),
            transitions: Vec::new(),
            change_requests: Vec::new(),
            as_of: None,
            next_message_cursor: None,
        })
        .expect("assemble")
    }

    #[test]
    fn elision_steps_in_blocks_when_a_block_fits() {
        let messages: Vec<Message> = (0..20)
            .map(|i| fixed_message(100 + i, &"m".repeat(80)))
            .collect();
        let per = message_tokens(&messages[0]);
        let (kept, elision) = fold_messages_to_budget(messages, per * 4);
        let elision = elision.expect("folded");
        assert_eq!(elision.elided_message_count % ELISION_BLOCK_MESSAGES, 0);
        assert_eq!(kept.len(), 20 - elision.elided_message_count);
        assert_eq!(kept[0].id, MessageId(uid(100)));
    }

    #[test]
    fn a_new_message_changes_no_byte_before_the_messages_layer() {
        let first = pack_with(vec![fixed_message(10, "one")], 5);
        let second = pack_with(vec![fixed_message(10, "one"), fixed_message(11, "two")], 9);
        assert!(first.prefix.starts_with_boot().expect("boot"));
        assert_eq!(
            first.bytes_before_messages().expect("before"),
            second.bytes_before_messages().expect("before")
        );
        assert_ne!(
            first.canonical_bytes().expect("bytes"),
            second.canonical_bytes().expect("bytes")
        );
    }

    fn rehash(pack: &mut ThreadContext) {
        let raw = serde_json::to_vec(&pack.prefix).expect("prefix");
        pack.tail.prefix_sha256 = sha256_hex(&raw);
        pack.tail.prefix_bytes = raw.len() as u64;
    }

    #[test]
    fn a_matching_prefix_sha_is_a_tail_only_delta() {
        let packed = pack_with(vec![fixed_message(10, "one")], 5);
        let delta = frame_delta(&packed, &packed, Some(&packed.tail.prefix_sha256), None);
        assert!(delta.prefix_unchanged);
        let (head, tail) = delta.parts().expect("parts");
        assert!(!head.is_empty());
        assert!(head.contains("\"prefix_unchanged\":true"));
        assert!(!head.contains("\"messages\""));
        assert!(tail.contains("\"state\""));
        let changed = frame_delta(&packed, &packed, Some("nope"), None);
        assert!(changed.prefix.is_some());
        assert!(changed.messages.is_none());
    }

    #[test]
    fn a_cursor_delta_appends_messages_when_that_rebuilds_the_prefix() {
        let older = pack_with(vec![fixed_message(10, "one")], 5);
        let newer = pack_with(vec![fixed_message(10, "one"), fixed_message(11, "two")], 9);
        let cursor = older.prefix.messages[0].id;
        let paged = pack_with(vec![fixed_message(11, "two")], 9);
        let delta = frame_delta(
            &newer,
            &paged,
            Some(&older.tail.prefix_sha256),
            Some(cursor),
        );
        assert!(!delta.prefix_unchanged);
        assert!(delta.prefix.is_none());
        let (head, _) = delta.parts().expect("parts");
        assert!(head.contains("\"messages\""));
        assert!(!head.contains("\"prefix\":{"));
        let messages = delta.messages.expect("suffix");
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].id, MessageId(uid(11)));
    }

    #[test]
    fn a_cursor_delta_replaces_the_prefix_when_stable_layers_changed() {
        let older = pack_with(vec![fixed_message(10, "one")], 5);
        let mut newer = pack_with(vec![fixed_message(10, "one"), fixed_message(11, "two")], 9);
        newer.prefix.thread.title = Some("renamed".into());
        rehash(&mut newer);
        let cursor = older.prefix.messages[0].id;
        let paged = pack_with(vec![fixed_message(11, "two")], 9);
        let delta = frame_delta(
            &newer,
            &paged,
            Some(&older.tail.prefix_sha256),
            Some(cursor),
        );
        assert!(delta.messages.is_none());
        assert!(delta.prefix.is_some());
        assert!(delta.since_message_cursor.is_none());
    }

    #[test]
    fn a_cursor_without_a_prefix_sha_is_the_message_slice() {
        let older = pack_with(vec![fixed_message(10, "one")], 5);
        let mut newer = pack_with(vec![fixed_message(10, "one"), fixed_message(11, "two")], 9);
        newer.prefix.thread.title = Some("renamed".into());
        rehash(&mut newer);
        let cursor = older.prefix.messages[0].id;
        let paged = pack_with(vec![fixed_message(11, "two")], 9);
        let delta = frame_delta(&newer, &paged, None, Some(cursor));
        assert!(delta.prefix.is_none());
        assert!(delta.messages.is_some());
    }

    #[test]
    fn a_new_reference_on_the_suffix_is_not_an_append() {
        let older = pack_with(vec![fixed_message(10, "one")], 5);
        let mut newer = pack_with(vec![fixed_message(10, "one"), fixed_message(11, "two")], 9);
        newer.prefix.references.push(crate::models::Reference {
            id: uid(50),
            src_kind: crate::models::RefSide::Message,
            src_id: uid(11),
            dst_kind: crate::models::RefSide::Thread,
            dst_id: uid(1),
            relation: crate::models::RelationKind::Supports,
            created_at: at_secs(11),
        });
        rehash(&mut newer);
        let cursor = older.prefix.messages[0].id;
        let paged = pack_with(vec![fixed_message(11, "two")], 9);
        let delta = frame_delta(
            &newer,
            &paged,
            Some(&older.tail.prefix_sha256),
            Some(cursor),
        );
        assert!(delta.messages.is_none());
        let prefix = delta.prefix.expect("replacement");
        assert_eq!(prefix.references.len(), 1);
    }

    #[test]
    fn context_pack_bytes_match_the_golden() {
        let packed = pack_with(vec![fixed_message(10, "one")], 5);
        let bytes = packed.canonical_bytes().expect("bytes");
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/context-pack-golden.json");
        if std::env::var_os("MAIDAN_WRITE_PORTABLE_GOLDENS").is_some() {
            std::fs::create_dir_all(path.parent().expect("dir")).expect("mkdir");
            std::fs::write(&path, &bytes).expect("write");
        }
        let golden = std::fs::read(&path).unwrap_or_default();
        assert_eq!(
            bytes, golden,
            "regenerate with MAIDAN_WRITE_PORTABLE_GOLDENS=1"
        );
    }
}

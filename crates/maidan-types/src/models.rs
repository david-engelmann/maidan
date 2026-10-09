//! Domain models. Each `<X>` has a paired `New<X>` for inserts so the
//! caller can build state-less values without populating server-assigned
//! fields (id, timestamps).

use crate::EventKind;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum MemberKind {
    Human,
    Agent,
}

impl MemberKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Human => "human",
            Self::Agent => "agent",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ThreadState {
    Open,
    InReview,
    Closed,
    Archived,
}

/// An agent's self-reported status on a thread. Distinct from [`ThreadState`]
/// (the workflow FSM) — this is what the agent says it's doing. `Stalled` is
/// system-computed only and cannot be declared.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum DeclaredStatus {
    Working,
    NeedsInput,
    NeedsReview,
    Blocked,
    Done,
}

impl DeclaredStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Working => "working",
            Self::NeedsInput => "needs_input",
            Self::NeedsReview => "needs_review",
            Self::Blocked => "blocked",
            Self::Done => "done",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "working" => Some(Self::Working),
            "needs_input" => Some(Self::NeedsInput),
            "needs_review" => Some(Self::NeedsReview),
            "blocked" => Some(Self::Blocked),
            "done" => Some(Self::Done),
            // "stalled" is system-computed only; refusing it here is the
            // enforcement.
            _ => None,
        }
    }

    /// Every declarable variant. Kept in sync by the exhaustive-match tripwire
    /// in the `all_variants_round_trip` test (`tests/declared_status.rs`).
    pub const ALL: &'static [Self] = &[
        Self::Working,
        Self::NeedsInput,
        Self::NeedsReview,
        Self::Blocked,
        Self::Done,
    ];
}

/// The longest note a status declaration may carry, in characters. A
/// declaration says in one sentence what the agent is doing; every board card
/// and every `StatusDeclared` event carries the note, so it stays short.
pub const STATUS_NOTE_MAX_CHARS: usize = 280;

/// A declaration's note, trimmed, or why it is refused: empty, more than one
/// line, or longer than [`STATUS_NOTE_MAX_CHARS`]. REST and MCP both call
/// this, so the two surfaces cannot drift.
/// A review's note, trimmed, with an empty one counted as none. A change
/// request names the change it asks for, so `request_changes` needs one: a
/// verdict that sends work back without saying why leaves the worker guessing.
pub fn review_note(
    decision: crate::ReviewDecision,
    note: Option<&str>,
) -> Result<Option<String>, String> {
    let note = note.map(str::trim).filter(|n| !n.is_empty());
    if decision == crate::ReviewDecision::RequestChanges && note.is_none() {
        return Err("request_changes needs a note saying what to change".into());
    }
    Ok(note.map(str::to_string))
}

/// The evidence root a verdict is bound to, lowercase. An approval names the
/// root of the review packet it was shown (`get_review_packet`), so it can
/// only count for that evidence; a change request may name one.
pub fn review_evidence(
    decision: crate::ReviewDecision,
    evidence_root: Option<&str>,
) -> Result<Option<String>, String> {
    let root = evidence_root
        .map(str::trim)
        .filter(|r| !r.is_empty())
        .map(str::to_ascii_lowercase);
    if let Some(r) = &root {
        if r.len() != 64 || !r.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(
                "evidence_root must be the 64-hex-character root from get_review_packet".into(),
            );
        }
    }
    if decision == crate::ReviewDecision::Approve && root.is_none() {
        return Err(
            "approve needs the evidence_root it approves: read get_review_packet and pass its evidence_root"
                .into(),
        );
    }
    Ok(root)
}

pub fn status_note(note: &str) -> Result<String, String> {
    let note = note.trim();
    if note.is_empty() {
        return Err("note must be a non-empty one-sentence description".into());
    }
    // Every character that ends a line: LF, CR, NEL and the Unicode line and
    // paragraph separators.
    if note
        .chars()
        .any(|c| matches!(c, '\n' | '\r' | '\u{85}' | '\u{2028}' | '\u{2029}'))
    {
        return Err("note must be a single sentence (no newlines)".into());
    }
    let chars = note.chars().count();
    if chars > STATUS_NOTE_MAX_CHARS {
        return Err(format!(
            "note must be at most {STATUS_NOTE_MAX_CHARS} characters (it is {chars})"
        ));
    }
    Ok(note.to_string())
}

#[cfg(test)]
mod status_note_tests {
    use super::*;

    #[test]
    fn a_status_note_is_one_trimmed_line_of_at_most_280_characters() {
        assert_eq!(
            status_note("  Writing the parser.  ").unwrap(),
            "Writing the parser."
        );
        assert!(status_note("   ").is_err());
        assert!(status_note("One.\nTwo.").is_err());
        assert!(status_note("One.\rTwo.").is_err());
        assert!(status_note("One.\u{85}Two.").is_err());
        assert!(status_note("One.\u{2028}Two.").is_err());
        assert!(status_note("One.\u{2029}Two.").is_err());
        assert!(
            status_note(&"é".repeat(STATUS_NOTE_MAX_CHARS)).is_ok(),
            "counted in characters, not bytes"
        );
        assert!(status_note(&"a".repeat(STATUS_NOTE_MAX_CHARS + 1)).is_err());
    }
}

/// An agent's status declaration on a thread: what it's doing, in one
/// sentence. Set by the claim holder or owner via `declare_status`; cleared
/// on a human response or a superseding declaration.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ThreadStatusDeclaration {
    pub thread_id: ThreadId,
    pub status: DeclaredStatus,
    pub note: String,
    pub declared_by: MemberId,
    pub declared_at: DateTime<Utc>,
}

/// A thread's version: how many writes its content has seen. The database
/// bumps it on every write to the thread's messages, result, title or
/// description, or linked artifacts, so a decision can name the version it was
/// shown. A thread nothing has written to is at 0.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ThreadVersion {
    pub thread_id: ThreadId,
    pub version: i64,
}

/// An artifact linked to a thread as evidence, by content hash.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ThreadArtifact {
    pub thread_id: ThreadId,
    pub sha256: String,
    pub linked_by: MemberId,
    pub linked_at: DateTime<Utc>,
}

impl ThreadState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::InReview => "in_review",
            Self::Closed => "closed",
            Self::Archived => "archived",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "open" => Some(Self::Open),
            "in_review" => Some(Self::InReview),
            "closed" => Some(Self::Closed),
            "archived" => Some(Self::Archived),
            _ => None,
        }
    }

    /// A terminal state — no further transitions, so a task in it counts as
    /// done for dependency readiness.
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Closed | Self::Archived)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum RefSide {
    Thread,
    Message,
}

impl RefSide {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Thread => "thread",
            Self::Message => "message",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ArtifactKind {
    Screenshot,
    Recording,
    Transcript,
    CodeDump,
    Attachment,
    /// A frozen, content-addressed context pack — tamper-evident "exactly what
    /// the agent was handed".
    ContextSnapshot,
}

impl ArtifactKind {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Screenshot => "screenshot",
            Self::Recording => "recording",
            Self::Transcript => "transcript",
            Self::CodeDump => "code_dump",
            Self::Attachment => "attachment",
            Self::ContextSnapshot => "context_snapshot",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "screenshot" => Some(Self::Screenshot),
            "recording" => Some(Self::Recording),
            "transcript" => Some(Self::Transcript),
            "code_dump" => Some(Self::CodeDump),
            "attachment" => Some(Self::Attachment),
            "context_snapshot" => Some(Self::ContextSnapshot),
            _ => None,
        }
    }

    pub fn default_mime(self) -> &'static str {
        match self {
            Self::Screenshot => "image/png",
            Self::Recording | Self::Attachment => "application/octet-stream",
            Self::Transcript | Self::CodeDump => "text/plain",
            Self::ContextSnapshot => "application/json",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Workspace {
    pub id: WorkspaceId,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub tombstoned_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct NewWorkspace {
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Member {
    pub id: MemberId,
    pub workspace_id: WorkspaceId,
    pub handle: String,
    pub display_name: Option<String>,
    pub kind: MemberKind,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub tombstoned_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct NewMember {
    pub workspace_id: WorkspaceId,
    pub handle: String,
    pub display_name: Option<String>,
    pub kind: MemberKind,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Channel {
    pub id: ChannelId,
    pub workspace_id: WorkspaceId,
    pub name: String,
    pub topic: Option<String>,
    pub private: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub tombstoned_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct NewChannel {
    pub workspace_id: WorkspaceId,
    pub name: String,
    pub topic: Option<String>,
    pub private: bool,
}

/// A member's role within a channel. `Admin` may manage membership; both roles
/// grant access to a private channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ChannelMemberRole {
    Member,
    Admin,
}

impl ChannelMemberRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Member => "member",
            Self::Admin => "admin",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "member" => Some(Self::Member),
            "admin" => Some(Self::Admin),
            _ => None,
        }
    }
}

/// Membership row for a channel. Rows exist for private channels; public
/// channels are open to the whole workspace without rows.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ChannelMember {
    pub channel_id: ChannelId,
    pub member_id: MemberId,
    pub role: ChannelMemberRole,
    pub created_at: DateTime<Utc>,
}

/// A free-form skill tag a member (agent) declares. Skill routing matches a
/// task's required skills against a member's declared skills.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MemberSkill {
    pub member_id: MemberId,
    pub skill: String,
    pub created_at: DateTime<Utc>,
}

/// A skill a task (thread) requires. A task is claimable by a member only if
/// every required skill is one the member has declared.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ThreadRequiredSkill {
    pub thread_id: ThreadId,
    pub skill: String,
    pub created_at: DateTime<Utc>,
}

/// The structured result an agent attaches to a task when it's done. One per
/// thread (a re-set overwrites). A requester — or a parent task that depends on
/// it — reads this back; coordination waits block on it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ThreadResult {
    pub thread_id: ThreadId,
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub result: serde_json::Value,
    pub produced_by: MemberId,
    pub produced_at: DateTime<Utc>,
}

/// The search-facet value of a result payload.
///
/// `result_kind` is a **namespaced string** (e.g. `example.review.result/1`),
/// not a closed enum — a waiter product ships a new kind without a Maidan
/// release. Missing, empty, whitespace-only, or non-string values are `None`
/// (the row is stored but not facetable under a kind). Does **not** require
/// `schema = "maidan.waiter.result/1"`: the facet is the string, not the
/// envelope.
pub fn result_kind_from_payload(value: &serde_json::Value) -> Option<&str> {
    value
        .get("result_kind")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
}

/// Abuse cap on a producer `run_id` / Maidan `parent_run_id`. The fixture is a
/// UUID (36 bytes); this is not a format rule — lineage accepts the producer's
/// string as-is, up to this length.
pub const PARENT_RUN_ID_MAX_BYTES: usize = 256;

/// The producer's `run_id` from an opaque result payload.
///
/// Same extractor shape as [`result_kind_from_payload`]: a **string, not a
/// minted id**. Missing, empty, whitespace-only, or non-string values are
/// `None`. Does **not** require `schema = "maidan.waiter.result/1"` — any
/// producer that writes `run_id` is first-class. Maidan homes the returned
/// value as [`ThreadLineage::parent_run_id`]; it never mints a parallel id.
pub fn run_id_from_payload(value: &serde_json::Value) -> Option<&str> {
    value
        .get("run_id")
        .and_then(|v| v.as_str())
        .and_then(normalize_parent_run_id)
}

/// Trim and accept a producer run identifier for lineage.
///
/// Empty / whitespace / longer than [`PARENT_RUN_ID_MAX_BYTES`] → `None`. Not a
/// UUID parse — the field accepts the producer's value.
pub fn normalize_parent_run_id(raw: &str) -> Option<&str> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || trimmed.len() > PARENT_RUN_ID_MAX_BYTES {
        return None;
    }
    Some(trimmed)
}

/// A thread's run lineage.
///
/// `parent_run_id` is the **producer's** run identifier — the same string the
/// producer puts on the waiter envelope as `run_id`. Nested threads that share this
/// value are attributed together for occupancy. F7 thread mute is orthogonal: a
/// mute never writes or clears this row.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ThreadLineage {
    pub thread_id: ThreadId,
    pub parent_run_id: String,
    pub set_at: DateTime<Utc>,
}

/// Occupancy of every **open** thread that shares a `parent_run_id` — the
/// nested-attribution view of [`ChannelOccupancy`].
///
/// Same four buckets (`queued` / `claimed` / `working` / `blocked` partition
/// `open`). Scoped to a workspace so two tenants cannot collide on a producer
/// id. F7 mute is not consulted: muted nested work still counts.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct RunOccupancy {
    pub parent_run_id: String,
    pub open: i64,
    pub queued: i64,
    pub claimed: i64,
    pub working: i64,
    pub blocked: i64,
}

/// A terminal thread's recorded result, as listed for a channel's claimer pack.
/// Store-level row: closed/archived, non-tombstoned, newest first. The pack
/// assembler (REST/MCP) projects this into a token-lean view and drops waiter
/// envelopes that are not `reviewed`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ChannelClosedResult {
    pub thread_id: ThreadId,
    pub title: Option<String>,
    pub state: ThreadState,
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub result: serde_json::Value,
    pub produced_by: MemberId,
    pub produced_at: DateTime<Utc>,
}

/// A thread parked from dispatch: while this exists, `claim_next` skips the
/// thread and an explicit `claim` is refused, until it is cleared. An explicit
/// human/owner park (needs triage, waiting on external, broken) — distinct from
/// blocked-by-deps, blocked-by-gate, and skill-miss.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ThreadUnclaimable {
    pub thread_id: ThreadId,
    pub reason: String,
    pub marked_by: MemberId,
    pub marked_at: DateTime<Utc>,
}

/// Why a thread is explicitly blocked from dispatch. A **closed** enum — unlike
/// `result_kind`, which is a namespaced string a producer publishes. Presence
/// of a [`ThreadBlock`] row is the block; absence is unblocked. Distinct/218
/// DAG readiness (children / deps must be terminal before `claim_next` will
/// pick a thread): that skip is derived from the dependency graph. This reason
/// is an orchestrator- set taxonomy of *why* a thread is parked from the queue.
///
/// `child` is "waiting on a child the orchestrator named", not "every DAG child
/// must be terminal". `unclaimable` here is the same vocabulary as the park, as
/// one of six reasons — the 363 side table is unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum BlockedReason {
    Dag,
    Gate,
    Human,
    Child,
    Quota,
    Unclaimable,
}

impl BlockedReason {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Dag => "dag",
            Self::Gate => "gate",
            Self::Human => "human",
            Self::Child => "child",
            Self::Quota => "quota",
            Self::Unclaimable => "unclaimable",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "dag" => Some(Self::Dag),
            "gate" => Some(Self::Gate),
            "human" => Some(Self::Human),
            "child" => Some(Self::Child),
            "quota" => Some(Self::Quota),
            "unclaimable" => Some(Self::Unclaimable),
            _ => None,
        }
    }

    /// Every variant. Kept in sync by the exhaustive-match tripwire in
    /// `blocked_reason_tests::all_variants_round_trip`.
    pub const ALL: &'static [Self] = &[
        Self::Dag,
        Self::Gate,
        Self::Human,
        Self::Child,
        Self::Quota,
        Self::Unclaimable,
    ];
}

/// An explicit dispatch block on a thread: while this exists, `claim_next`
/// skips the thread. One block per thread (upsert). Clearing the row is the
/// unblock — later clusters emit `BlockedResolved`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ThreadBlock {
    pub thread_id: ThreadId,
    pub reason: BlockedReason,
    pub set_by: MemberId,
    pub set_at: DateTime<Utc>,
    /// Human-readable note explaining why the thread is blocked. Set by
    /// `set_thread_block`; empty when the blocker gave no note.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

/// A per-thread budget envelope. An orchestrator sets any of the optional
/// maxima; an agent reports incremental usage as it works, and when a dimension
/// is exceeded the run is stopped (the claim fails → DLQ). USD is integer
/// micros ($1 = 1_000_000) to keep money out of floats. Wall time is measured,
/// never reported: the live claim's share comes from the thread's working clock
/// (`work_started_at`), and `used_wall_secs` holds what earlier claims worked
/// before they ended. A lapsed lease is charged through its deadline; every
/// other ending is charged through the moment it ended.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ThreadBudget {
    pub thread_id: ThreadId,
    pub max_tokens: Option<i64>,
    pub max_usd_micros: Option<i64>,
    pub max_turns: Option<i64>,
    pub max_wall_secs: Option<i64>,
    pub used_tokens: i64,
    pub used_usd_micros: i64,
    pub used_turns: i64,
    /// Seconds worked by earlier claims on this thread, from each one's
    /// acknowledgement to the moment that claim ended. A lapsed lease ends at
    /// its deadline. Counted against `max_wall_secs` together with the live
    /// claim's working time.
    #[serde(default)]
    pub used_wall_secs: i64,
    /// Uncached input charged on this thread. `used_tokens` is the fresh sum,
    /// not this column.
    #[serde(default)]
    pub used_input_tokens: i64,
    #[serde(default)]
    pub used_output_tokens: i64,
    /// Cache reads. Counted in `used_usd_micros` at their price, not in
    /// `used_tokens` / `max_tokens`.
    #[serde(default)]
    pub used_cache_read_tokens: i64,
    #[serde(default)]
    pub used_cache_write_5m_tokens: i64,
    #[serde(default)]
    pub used_cache_write_1h_tokens: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// The maxima an orchestrator sets on a thread's budget. Each dimension is
/// optional — set the ones you want to bind; omit (or `None`) leaves that
/// dimension unbounded. Does not touch accumulated usage.
///
/// **Unknown fields are rejected**. The write is a replace, so
/// omission is load-bearing: leaving a dimension out *removes* that limit. That
/// makes a misspelled key indistinguishable from a deliberate omission — send
/// `max_wall_seconds` instead of `max_wall_secs` and the wall cap is silently
/// dropped, with a `200` and the budget echoed back. Of everywhere in this
/// codebase that absorbs an unknown field, this is the one where a typo disarms
/// a safety control, so here the strictness is worth the rigidity.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct BudgetLimits {
    #[serde(default)]
    pub max_tokens: Option<i64>,
    #[serde(default)]
    pub max_usd_micros: Option<i64>,
    #[serde(default)]
    pub max_turns: Option<i64>,
    #[serde(default)]
    pub max_wall_secs: Option<i64>,
}

/// A partial change to a thread's budget.
///
/// [`BudgetLimits`] is a **total replace**: every dimension it does not name
/// becomes "no cap", and a dimension with no cap never binds. So sending
/// `{max_tokens}` to raise one limit silently removed the usd, turns and wall
/// limits — and a removed limit is a run that should have been stopped and was
/// not. the `deny_unknown_fields` catches a *typo*; it cannot catch a
/// well-formed body that simply omits a field.
///
/// Each field here distinguishes three states:
///
/// * **absent** — leave this dimension exactly as it is
/// * **`null`** — clear the cap on this dimension (explicitly unbounded)
/// * **a value** — set it
///
/// So widening always requires *saying so*: there is no spelling of a budget
/// change that removes a cap by omission.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(deny_unknown_fields)]
pub struct BudgetPatch {
    #[serde(default, deserialize_with = "double_option")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<i64>, nullable))]
    pub max_tokens: Option<Option<i64>>,
    #[serde(default, deserialize_with = "double_option")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<i64>, nullable))]
    pub max_usd_micros: Option<Option<i64>>,
    #[serde(default, deserialize_with = "double_option")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<i64>, nullable))]
    pub max_turns: Option<Option<i64>>,
    #[serde(default, deserialize_with = "double_option")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<i64>, nullable))]
    pub max_wall_secs: Option<Option<i64>>,
}

/// Tell "absent" from "explicitly null".
///
/// Serde collapses both to `None` for a plain `Option<T>` — a missing field is
/// silently `None` — which is exactly the collapse a partial update must undo.
fn double_option<'de, T, D>(de: D) -> Result<Option<Option<T>>, D::Error>
where
    T: Deserialize<'de>,
    D: serde::Deserializer<'de>,
{
    Deserialize::deserialize(de).map(Some)
}

impl BudgetPatch {
    /// Apply this patch to the dimensions currently stored.
    ///
    /// Pure, so the merge rule is unit-testable without a database and the two
    /// backends cannot disagree about it.
    pub fn apply(&self, current: BudgetLimits) -> BudgetLimits {
        BudgetLimits {
            max_tokens: self.max_tokens.unwrap_or(current.max_tokens),
            max_usd_micros: self.max_usd_micros.unwrap_or(current.max_usd_micros),
            max_turns: self.max_turns.unwrap_or(current.max_turns),
            max_wall_secs: self.max_wall_secs.unwrap_or(current.max_wall_secs),
        }
    }

    /// Dimensions this patch does not mention, by wire name.
    ///
    /// A **total replace** uses this to refuse a body that would clear a cap by
    /// omission — and to say *which* cap, because "send all four" is a worse
    /// error than naming the one you forgot.
    pub fn missing_dimensions(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.max_tokens.is_none() {
            out.push("max_tokens");
        }
        if self.max_usd_micros.is_none() {
            out.push("max_usd_micros");
        }
        if self.max_turns.is_none() {
            out.push("max_turns");
        }
        if self.max_wall_secs.is_none() {
            out.push("max_wall_secs");
        }
        out
    }

    /// True when the patch names no dimension at all.
    pub fn is_empty(&self) -> bool {
        self.missing_dimensions().len() == 4
    }
}

#[cfg(test)]
mod budget_patch_tests {
    use super::*;

    fn current() -> BudgetLimits {
        BudgetLimits {
            max_tokens: Some(100),
            max_usd_micros: Some(200),
            max_turns: Some(3),
            max_wall_secs: Some(400),
        }
    }

    /// The defect this type exists for: raising one cap used to clear the rest.
    #[test]
    fn naming_one_dimension_leaves_the_others_alone() {
        let patch: BudgetPatch = serde_json::from_str(r#"{"max_tokens": 999}"#).unwrap();
        let merged = patch.apply(current());
        assert_eq!(merged.max_tokens, Some(999));
        assert_eq!(merged.max_usd_micros, Some(200), "usd cap must survive");
        assert_eq!(merged.max_turns, Some(3), "turns cap must survive");
        assert_eq!(merged.max_wall_secs, Some(400), "wall cap must survive");
    }

    /// Clearing is still possible — it just has to be said.
    #[test]
    fn an_explicit_null_clears_that_dimension() {
        let patch: BudgetPatch = serde_json::from_str(r#"{"max_usd_micros": null}"#).unwrap();
        let merged = patch.apply(current());
        assert_eq!(merged.max_usd_micros, None, "explicit null clears");
        assert_eq!(merged.max_tokens, Some(100), "and only that one");
    }

    /// Absent and null must not collapse. Serde *does* collapse them for a plain
    /// `Option<T>`, which is why `double_option` exists — if this regresses, a
    /// partial update silently becomes a total replace again, which is the whole
    /// defect.
    #[test]
    fn absent_and_null_are_different() {
        let absent: BudgetPatch = serde_json::from_str("{}").unwrap();
        let null: BudgetPatch = serde_json::from_str(r#"{"max_tokens": null}"#).unwrap();
        assert_eq!(absent.max_tokens, None, "absent");
        assert_eq!(null.max_tokens, Some(None), "explicitly null");
        assert!(absent.is_empty());
        assert!(!null.is_empty());
        assert_eq!(
            absent.apply(current()),
            current(),
            "an empty patch is a no-op"
        );
        assert_eq!(null.apply(current()).max_tokens, None);
    }

    /// A typo cannot masquerade as a dimension.
    #[test]
    fn an_unknown_field_is_rejected() {
        assert!(
            serde_json::from_str::<BudgetPatch>(r#"{"max_wall_seconds": 5}"#).is_err(),
            "a misspelled dimension must not be absorbed"
        );
    }

    /// A total replace names what a partial body left out, so the caller is told
    /// which cap they were about to clear rather than "send all four".
    #[test]
    fn a_total_replace_can_name_what_is_missing() {
        let partial: BudgetPatch = serde_json::from_str(r#"{"max_tokens": 1}"#).unwrap();
        assert_eq!(
            partial.missing_dimensions(),
            vec!["max_usd_micros", "max_turns", "max_wall_secs"]
        );
        let full: BudgetPatch = serde_json::from_str(
            r#"{"max_tokens":1,"max_usd_micros":null,"max_turns":null,"max_wall_secs":null}"#,
        )
        .unwrap();
        assert!(full.missing_dimensions().is_empty());
        let limits = full.apply(BudgetLimits::default());
        assert_eq!(limits.max_tokens, Some(1));
        assert_eq!(limits.max_usd_micros, None, "an explicit null is no cap");
    }
}

/// An increment of resource usage an agent reports against a thread's budget.
/// Each dimension defaults to 0.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UsageDelta {
    #[serde(default)]
    pub tokens: i64,
    #[serde(default)]
    pub usd_micros: i64,
    #[serde(default)]
    pub turns: i64,
}

/// The outcome of reporting usage against a thread's budget. Always carries the
/// new totals; `stopped` is true when this report pushed the thread over budget
/// and its claimed run was stopped (claim released + `ClaimFailed` + DLQ), with
/// `reason` the dimension that bound.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UsageReport {
    pub budget: ThreadBudget,
    pub stopped: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Which budget dimension was exceeded — the reason a run was stopped, carried
/// on the `ClaimFailed` event and the DLQ entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetReason {
    Tokens,
    Usd,
    Turns,
    Wall,
}

impl BudgetReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tokens => "tokens",
            Self::Usd => "usd",
            Self::Turns => "turns",
            Self::Wall => "wall",
        }
    }
}

impl ThreadBudget {
    /// The first budget dimension exceeded, if any — checked in a fixed order
    /// (tokens, usd, turns, wall). `wall_secs_elapsed` is the live claim's
    /// working-clock elapsed time; pass `None` when no claim is working, and
    /// the wall dimension is then the time already charged
    /// (`used_wall_secs`) alone. A dimension with no maximum, or a
    /// non-positive maximum, never binds.
    pub fn exceeded(&self, wall_secs_elapsed: Option<i64>) -> Option<BudgetReason> {
        let bound = |used: i64, max: Option<i64>| max.is_some_and(|m| m > 0 && used >= m);
        if bound(self.used_tokens, self.max_tokens) {
            return Some(BudgetReason::Tokens);
        }
        if bound(self.used_usd_micros, self.max_usd_micros) {
            return Some(BudgetReason::Usd);
        }
        if bound(self.used_turns, self.max_turns) {
            return Some(BudgetReason::Turns);
        }
        let worked = self
            .used_wall_secs
            .saturating_add(wall_secs_elapsed.unwrap_or(0).max(0));
        if bound(worked, self.max_wall_secs) {
            return Some(BudgetReason::Wall);
        }
        None
    }
}

/// A member's notifications for one thread, collapsed. The grouped inbox shows
/// one row per thread — the newest notification plus how many (and how many
/// unread) it stands for — so a busy thread doesn't flood the flat list.
/// `thread_id` is `None` for the group of notifications that carry no thread.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct NotificationThreadGroup {
    pub thread_id: Option<ThreadId>,
    pub count: i64,
    pub unread_count: i64,
    /// The newest notification in the group (its `created_at` orders the groups).
    pub latest: Notification,
}

/// Collapse a member's notifications into per-thread groups, newest-activity
/// first. Each group's `latest` is its most recent notification; groups are
/// ordered by that notification's `created_at` (descending). The input is
/// assumed newest-first (as [`Notification`] lists are), so the first
/// notification seen for a thread is its latest. Pure — the caller fetches the
/// list (already snooze-filtered) and groups it.
pub fn group_notifications_by_thread(
    notifications: &[Notification],
) -> Vec<NotificationThreadGroup> {
    let mut order: Vec<Option<ThreadId>> = Vec::new();
    let mut groups: std::collections::HashMap<Option<ThreadId>, NotificationThreadGroup> =
        std::collections::HashMap::new();
    for n in notifications {
        let entry = groups.entry(n.thread_id).or_insert_with(|| {
            order.push(n.thread_id);
            NotificationThreadGroup {
                thread_id: n.thread_id,
                count: 0,
                unread_count: 0,
                latest: n.clone(),
            }
        });
        entry.count += 1;
        if n.read_at.is_none() {
            entry.unread_count += 1;
        }
        if n.created_at > entry.latest.created_at {
            entry.latest = n.clone();
        }
    }
    let mut out: Vec<NotificationThreadGroup> = order
        .into_iter()
        .filter_map(|k| groups.remove(&k))
        .collect();
    out.sort_by(|a, b| b.latest.created_at.cmp(&a.latest.created_at));
    out
}

/// A dead-lettered agent run. When a claimed run is stopped because it exceeded
/// its budget envelope, the claim fails and a DLQ entry is recorded — so the
/// failed work is triageable (retry, raise the budget, give up) rather than
/// silently lost or silently marked done. Captures the failure snapshot: which
/// thread, which agent, why, and usage at the moment of failure.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DlqEntry {
    pub id: DlqEntryId,
    pub workspace_id: WorkspaceId,
    pub channel_id: ChannelId,
    pub thread_id: ThreadId,
    /// The agent whose run was stopped.
    pub member_id: MemberId,
    /// The budget dimension that bound (`BudgetReason::as_str`).
    pub reason: String,
    pub used_tokens: i64,
    pub used_usd_micros: i64,
    pub used_turns: i64,
    pub failed_at: DateTime<Utc>,
}

/// A new dead-letter entry to record. `id`/`failed_at` are assigned by the
/// store.
#[derive(Debug, Clone)]
pub struct NewDlqEntry {
    pub workspace_id: WorkspaceId,
    pub channel_id: ChannelId,
    pub thread_id: ThreadId,
    pub member_id: MemberId,
    pub reason: String,
    pub used_tokens: i64,
    pub used_usd_micros: i64,
    pub used_turns: i64,
}

/// Persisted steering guidance for a task/thread. A durable instruction from
/// the owner (or a supervisor) that survives claims and handoffs, so a resuming
/// or newly-assigned agent reads the CURRENT steer. One per thread (a re-set
/// overwrites). Distinct from a handoff note, which rides an assignment event
/// and is not persisted.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ThreadSteer {
    pub thread_id: ThreadId,
    pub steer: String,
    pub steered_by: MemberId,
    pub steered_at: DateTime<Utc>,
}

/// The state of an approval gate. A gate opens `Pending`; a human resolves it
/// to exactly one of accept/decline/cancel. Silence never resolves a gate
/// (there is no timeout auto-approve), and a resolve is a compare-and-set on
/// `Pending` so a double-answer can't flip it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ApprovalGateState {
    #[default]
    Pending,
    Accepted,
    Declined,
    Cancelled,
}

impl ApprovalGateState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Accepted => "accepted",
            Self::Declined => "declined",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "pending" => Some(Self::Pending),
            "accepted" => Some(Self::Accepted),
            "declined" => Some(Self::Declined),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }

    /// Whether the gate is no longer awaiting a human.
    pub fn is_resolved(self) -> bool {
        !matches!(self, Self::Pending)
    }
}

/// A durable, queryable human-approval gate. An agent's `request_approval`
/// opens one `Pending` gate and returns an `input-required` result instead of
/// blocking; a human later resolves it via the `/ui`. Persisted so the gate
/// survives a dropped connection and can be listed while outstanding
/// (queryable). An optional `thread_id` attaches the gate to a thread for the
/// N6 required-human claim gate.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ApprovalGate {
    pub id: ApprovalGateId,
    pub workspace_id: WorkspaceId,
    pub thread_id: Option<ThreadId>,
    pub requested_by: MemberId,
    pub prompt: String,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub schema: Option<serde_json::Value>,
    pub state: ApprovalGateState,
    #[cfg_attr(feature = "openapi", schema(value_type = Option<Object>))]
    pub content: Option<serde_json::Value>,
    pub resolved_by: Option<MemberId>,
    /// The delegate that actually opened the gate for `requested_by`, when one
    /// did. `None`: `requested_by` opened it itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_actor_id: Option<MemberId>,
    /// The delegate that actually answered it for `resolved_by`, when one did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_actor_id: Option<MemberId>,
    pub created_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
    /// How much a wrong accept would cost, as whoever opened the gate said.
    /// `high` when it said nothing.
    #[serde(default)]
    pub risk: ApprovalRisk,
    /// Set when a model decided the gate through `approval_decide`, directly
    /// or by a person confirming its request in the console. `None` for an
    /// answer given over REST or on the console's own buttons.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decided_via: Option<GateDecisionVia>,
}

/// The inputs to open a new approval gate.
#[derive(Debug, Clone)]
pub struct NewApprovalGate {
    pub workspace_id: WorkspaceId,
    pub thread_id: Option<ThreadId>,
    pub requested_by: MemberId,
    pub prompt: String,
    pub schema: Option<serde_json::Value>,
    pub risk: ApprovalRisk,
}

/// How much a wrong accept of a gate would cost. Ordered: `Low < Medium <
/// High`. A workspace's [`ApprovalPolicy`] names the lowest risk at which a
/// model's accept needs a person to confirm it.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default, Serialize, Deserialize,
)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ApprovalRisk {
    Low,
    Medium,
    /// The default: a gate that says nothing is treated as the costliest kind.
    #[default]
    High,
}

impl ApprovalRisk {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            _ => None,
        }
    }
}

/// Which MCP client a model decided a gate through, and that a model asked.
/// The client's name and version are what it called itself in
/// `clientInfo`: a label for the record, never a credential. `None` when the
/// request named no client.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct GateDecisionVia {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_version: Option<String>,
    /// Always `true` on a recorded decision: the tool is only ever called by
    /// a model. Kept as a field so the record says it, not the reader.
    pub model_asked: bool,
}

/// The confirmation threshold a workspace sets for `approval_decide`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ApprovalPolicy {
    pub workspace_id: WorkspaceId,
    /// The lowest gate risk at which a model's accept needs a person to
    /// confirm it in the console. `low` (the default) means every one does.
    pub confirm_at: ApprovalRisk,
    /// How long a confirmation link `approval_decide` sends lives, in
    /// seconds: from 60 to 3600, 600 (ten minutes) by default. A link keeps
    /// the lifetime it was sent with.
    pub confirm_link_ttl_seconds: u32,
    /// `true` when the workspace has set nothing and the defaults apply.
    pub is_default: bool,
}

impl ApprovalPolicy {
    /// Whether a model's accept of a gate at `risk` needs a confirmation.
    pub fn needs_confirmation(&self, risk: ApprovalRisk) -> bool {
        risk >= self.confirm_at
    }
}

/// A model's pending request to accept a gate, waiting for the person whose
/// credential it used to confirm it in the console. The token in the link is
/// derived from `nonce` and never stored; only its hash is.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ApprovalConfirmation {
    pub gate_id: ApprovalGateId,
    pub member_id: MemberId,
    pub workspace_id: WorkspaceId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor_id: Option<MemberId>,
    #[serde(skip)]
    pub nonce: uuid::Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub used_at: Option<DateTime<Utc>>,
}

impl ApprovalConfirmation {
    /// Unused and unexpired at `now`.
    pub fn is_live(&self, now: DateTime<Utc>) -> bool {
        self.used_at.is_none() && self.expires_at > now
    }
}

/// The inputs to issue (or find the live) confirmation for a gate and member.
#[derive(Debug, Clone)]
pub struct NewApprovalConfirmation {
    pub gate_id: ApprovalGateId,
    pub member_id: MemberId,
    pub workspace_id: WorkspaceId,
    pub actor_id: Option<MemberId>,
    pub nonce: uuid::Uuid,
    pub token_hash: String,
    pub client_name: Option<String>,
    pub client_version: Option<String>,
    pub note: Option<String>,
    pub now: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

/// What confirming a model's request came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfirmOutcome {
    /// The gate is accepted, recorded as decided via the requesting client.
    Accepted(Box<ApprovalGate>),
    /// No live confirmation matches: unknown, someone else's, used or
    /// expired. One answer, so nothing about another's link leaks.
    NotFound,
    /// The confirmation was live but the gate had already been resolved.
    GateResolved,
}

/// A per-recipient notification. Where a mention is one shared
/// `maidan_mentions` row read through a single inbox cursor, this is one row
/// per (recipient, source event): *who* should know, *what* triggered it
/// (`kind` = the source [`EventKind`] + `source_log_id` = the event-log row),
/// denormalized context (`channel/thread/message/actor`) so the inbox renders
/// without re-fetching the event, and per-recipient read state. The
/// zero-blast-radius foundation for the notification router + unified inbox
/// that follow — nothing writes rows yet.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Notification {
    pub id: NotificationId,
    pub workspace_id: WorkspaceId,
    /// The recipient.
    pub member_id: MemberId,
    pub kind: EventKind,
    /// The `maidan_events` row that triggered this notification.
    pub source_log_id: i64,
    pub channel_id: Option<ChannelId>,
    pub thread_id: Option<ThreadId>,
    pub message_id: Option<MessageId>,
    /// Who caused it (e.g. the mentioner), when applicable.
    pub actor_id: Option<MemberId>,
    pub created_at: DateTime<Utc>,
    /// `None` = unread.
    pub read_at: Option<DateTime<Utc>>,
    /// Snoozed until this instant — while in the future the notification is
    /// hidden from the default inbox + badge, then resurfaces. `None` = not
    /// snoozed. Orthogonal to `read_at`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snoozed_until: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct NewNotification {
    pub workspace_id: WorkspaceId,
    pub member_id: MemberId,
    pub kind: EventKind,
    pub source_log_id: i64,
    pub channel_id: Option<ChannelId>,
    pub thread_id: Option<ThreadId>,
    pub message_id: Option<MessageId>,
    pub actor_id: Option<MemberId>,
}

/// A member's notification preference for one event kind. `muted` suppresses
/// router-written notifications of `kind` for this member; the absence of a row
/// is the default (notify). The routing brain the notification router consults
/// before writing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct NotificationPref {
    pub member_id: MemberId,
    pub kind: EventKind,
    pub muted: bool,
    pub updated_at: DateTime<Utc>,
}

/// A member following a channel — presence = following. The notification router
/// notifies followers of activity in the channel, honoring mutes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ChannelFollow {
    pub member_id: MemberId,
    pub channel_id: ChannelId,
    pub created_at: DateTime<Utc>,
}

/// A member following a thread — presence = following.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ThreadFollow {
    pub member_id: MemberId,
    pub thread_id: ThreadId,
    pub created_at: DateTime<Utc>,
}

/// A member following another member's work occupancy. The follower receives
/// the followed member's relevant work-lifecycle notifications and may read a
/// live presence + assigned-work snapshot through the server surface. Presence
/// itself stays ephemeral; this row stores only the subscription edge.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MemberFollow {
    pub follower_id: MemberId,
    pub followed_id: MemberId,
    pub created_at: DateTime<Utc>,
}

/// Ephemeral presence reported in a member occupancy snapshot. `Offline` means
/// no live WebSocket presence is known on this replica or its presence peers;
/// it is not a durable event or historical fact.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum OccupancyPresence {
    Offline,
    Away,
    Online,
}

/// A followed member's live occupancy: ephemeral presence plus the currently
/// assigned, non-terminal work visible to the caller. Access filtering happens
/// at the transport boundary so private-channel threads never leak here.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MemberOccupancy {
    pub member_id: MemberId,
    pub presence: OccupancyPresence,
    pub assigned_threads: Vec<Thread>,
}

/// Bump when the inner export graph changes in a way an importer must notice.
pub const WORKSPACE_EXPORT_FORMAT_VERSION: u32 = 1;

/// Nested channel + members as assembled for export.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportChannel {
    pub channel: Channel,
    pub members: Vec<ChannelMember>,
}

/// Workspace content graph. Secrets are omitted — tokens die on export. This is
/// the signed envelope's `payload`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceExport {
    pub format_version: u32,
    pub exported_at: DateTime<Utc>,
    pub workspace: Workspace,
    pub members: Vec<Member>,
    pub channels: Vec<ExportChannel>,
    pub threads: Vec<Thread>,
    pub messages: Vec<Message>,
    pub message_edits: Vec<MessageEdit>,
    pub pins: Vec<Pin>,
    pub references: Vec<Reference>,
}

/// A workspace's content graph for import — the flat, id-linked collections of
/// an export bundle, ready to insert. The server flattens its `WorkspaceExport`
/// (which nests channel members under each channel) into this and optionally
/// remaps every id for a fresh-workspace import.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceImport {
    pub workspace: Workspace,
    pub members: Vec<Member>,
    pub channels: Vec<Channel>,
    pub channel_members: Vec<ChannelMember>,
    pub threads: Vec<Thread>,
    pub messages: Vec<Message>,
    pub message_edits: Vec<MessageEdit>,
    pub pins: Vec<Pin>,
    pub references: Vec<Reference>,
}

/// A member's delivery email address — where email notifications go. One per
/// member.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MemberEmail {
    pub member_id: MemberId,
    pub email: String,
    pub updated_at: DateTime<Utc>,
}

/// A claimed entry from the durable mail outbox the retry worker will attempt
/// to send. `attempts` includes the current claim. Content-only — the outbox's
/// status / scheduling columns stay internal to the store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MailOutbox {
    pub id: MailOutboxId,
    pub to_address: String,
    pub subject: String,
    pub body: String,
    pub attempts: i64,
}

/// A new outbound notification email to enqueue for durable, retryable
/// delivery. Enqueued `pending` with `next_attempt_at = now`.
#[derive(Debug, Clone)]
pub struct NewMailOutbox {
    /// Owning workspace. `None` only for mail with no tenant context; such a
    /// row is visible to `operator:global` alone, because it cannot be
    /// attributed to a caller's workspace.
    pub workspace_id: Option<WorkspaceId>,
    /// The event the mail is about. When it is a message event, the mail is
    /// linked to the message's content key and goes with it: withdrawing the
    /// message deletes the mail, and a mail about an already withdrawn message
    /// is not queued.
    pub source_log_id: Option<i64>,
    pub to_address: String,
    pub subject: String,
    pub body: String,
}

/// A dead-lettered outbox entry for the operator DLQ view: a message that
/// exhausted its retries. `last_error` is why the final attempt failed.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DeadMail {
    pub id: MailOutboxId,
    /// `None` for a pre-row or tenant-less mail.
    pub workspace_id: Option<WorkspaceId>,
    pub to_address: String,
    pub subject: String,
    pub attempts: i64,
    pub last_error: Option<String>,
    pub updated_at: DateTime<Utc>,
}

/// A Slack projector channel link: a Slack channel projects into the
/// `thread_id` in `channel_id`/`workspace_id`, with inbound Slack messages
/// posted as `member_id`. One Maidan thread per Slack channel.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SlackChannelLink {
    pub slack_channel_id: String,
    pub workspace_id: WorkspaceId,
    pub channel_id: ChannelId,
    pub thread_id: ThreadId,
    pub member_id: MemberId,
    pub created_at: DateTime<Utc>,
    /// When egress to this channel was disabled after an auth/config-class
    /// failure. `None` = enabled; re-linking clears it. Ingress is unaffected —
    /// a revoked *write* scope does not stop Slack from reaching us.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled_at: Option<DateTime<Utc>>,
}

/// A new Slack channel link to create.
#[derive(Debug, Clone)]
pub struct NewSlackChannelLink {
    pub slack_channel_id: String,
    pub workspace_id: WorkspaceId,
    pub channel_id: ChannelId,
    pub thread_id: ThreadId,
    pub member_id: MemberId,
}

/// A GitHub projector issue/PR link: a GitHub issue/PR (`repo` full-name +
/// `issue_number`) projects into the `thread_id` in
/// `channel_id`/`workspace_id`, with inbound comments posted as `member_id`.
/// One Maidan thread per GitHub issue/PR.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct GithubIssueLink {
    pub repo: String,
    pub issue_number: i64,
    pub workspace_id: WorkspaceId,
    pub channel_id: ChannelId,
    pub thread_id: ThreadId,
    pub member_id: MemberId,
    pub created_at: DateTime<Utc>,
    /// When egress to this issue/PR was disabled after an auth/config-class
    /// failure. `None` = enabled; re-linking clears it. Ingress is unaffected —
    /// the webhook keeps delivering comments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled_at: Option<DateTime<Utc>>,
}

/// A new GitHub issue/PR link to create.
#[derive(Debug, Clone)]
pub struct NewGithubIssueLink {
    pub repo: String,
    pub issue_number: i64,
    pub workspace_id: WorkspaceId,
    pub channel_id: ChannelId,
    pub thread_id: ThreadId,
    pub member_id: MemberId,
}

/// How a member wants notification emails delivered. The default (an absent
/// preference row) is `Immediate` — the behaviour. `Digest` opts out of
/// per-notification emails in favour of a periodic rollup from the digest
/// sweeper; the two are mutually exclusive by design.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum EmailDeliveryMode {
    #[default]
    Immediate,
    Digest,
}

impl EmailDeliveryMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Immediate => "immediate",
            Self::Digest => "digest",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "immediate" => Some(Self::Immediate),
            "digest" => Some(Self::Digest),
            _ => None,
        }
    }
}

/// What a thread wait does when its deadline lapses. **Never a decision** — the
/// "TimedOut ≠ Decline" rule: a timeout must not invent a human refusal (or
/// approval). Both variants emit a `WaitTimedOut` event (the notification
/// router then reaches the thread's owner); `Park` additionally marks the
/// thread unclaimable so `claim_next` won't dispatch a stuck thread until a
/// human intervenes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum EscalationPolicy {
    /// Emit `WaitTimedOut` (reach the owner). No thread state change.
    #[default]
    Notify,
    /// Emit `WaitTimedOut` + park the thread from dispatch (unclaimable).
    Park,
}

impl EscalationPolicy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Notify => "notify",
            Self::Park => "park",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "notify" => Some(Self::Notify),
            "park" => Some(Self::Park),
            _ => None,
        }
    }
}

/// A durable timer on a thread: the thread is waiting until `wait_until`, and
/// on timeout the `on_timeout` policy escalates. Either cancelled (satisfied —
/// the awaited thing happened) or fired by the sweeper (`fired_at` set). One
/// wait per thread. Steals the Restate/Temporal promise/timer
/// *shape* — this is not a workflow engine.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ThreadWait {
    pub thread_id: ThreadId,
    pub wait_until: DateTime<Utc>,
    pub on_timeout: EscalationPolicy,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub created_by: MemberId,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fired_at: Option<DateTime<Utc>>,
}

/// A thread's dispatch priority. Higher = more urgent; the default (no row) is
/// `0` (normal). `claim_next` orders by an effective rank = `priority` aged
/// upward the longer a thread has waited, so a high-priority task jumps the
/// queue while a long-waiting normal task is never starved. One priority per
/// thread.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ThreadPriority {
    pub thread_id: ThreadId,
    pub priority: i64,
    pub set_by: MemberId,
    pub set_at: DateTime<Utc>,
}

/// A legal hold on a workspace. While held, the workspace's event-log and audit
/// rows are exempt from retention pruning, workspace
/// purge/erase is refused, and a message withdrawn (tombstoned) keeps its words
/// and earlier versions in [`PreservedMessage`] — evidence is preserved for
/// litigation. A workspace may be held for several matters at once, one hold
/// each; it is held while any hold remains, and what the holds kept is disposed
/// of only when the last is lifted.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct LegalHold {
    pub id: LegalHoldId,
    pub workspace_id: WorkspaceId,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placed_by: Option<MemberId>,
    pub placed_at: DateTime<Utc>,
}

/// What a legal hold kept of a message withdrawn while the workspace was held:
/// its last words and every earlier version. The member's view is unchanged —
/// the message is gone everywhere they look. Read only through the audited
/// preserved-content read; lifting the hold deletes it.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PreservedMessage {
    pub message_id: MessageId,
    pub thread_id: ThreadId,
    pub channel_id: ChannelId,
    pub author_id: MemberId,
    pub posted_at: DateTime<Utc>,
    pub body: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<Vec<ContentBlock>>,
    pub tombstoned_at: DateTime<Utc>,
    /// Earlier versions, oldest first.
    pub edits: Vec<MessageEdit>,
}

/// A member's Web Push subscription — one browser/device. From the browser's
/// `PushManager.subscribe()`: `endpoint` is the push service URL, `p256dh` the
/// subscription's public ECDH key and `auth` its auth secret (both base64url).
/// The notification router delivers a Web Push message to `endpoint` when the
/// member has no live WebSocket connection.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PushSubscription {
    pub id: PushSubscriptionId,
    pub member_id: MemberId,
    pub endpoint: String,
    pub p256dh: String,
    pub auth: String,
    pub created_at: DateTime<Utc>,
}

/// A new Web Push subscription to register.
#[derive(Debug, Clone)]
pub struct NewPushSubscription {
    pub member_id: MemberId,
    pub endpoint: String,
    pub p256dh: String,
    pub auth: String,
}

/// A claimed web push retry. `attempts` includes the current claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebPushOutbox {
    pub id: WebPushOutboxId,
    pub member_id: MemberId,
    pub subscription_id: PushSubscriptionId,
    pub payload: String,
    pub attempts: i64,
}

/// A failed web push to retry. `attempts` counts tries already made, including
/// the send that just failed. `next_attempt_at` is when the worker may claim it.
#[derive(Debug, Clone)]
pub struct NewWebPushOutbox {
    pub member_id: MemberId,
    pub subscription_id: PushSubscriptionId,
    pub payload: String,
    pub attempts: i64,
    pub next_attempt_at: DateTime<Utc>,
    pub last_error: String,
}

/// SCIM 2.0 provisioning link for a member. Holds the SCIM-specific fields —
/// the IdP's `externalId` and the `active` flag — while `userName`/`id` map to
/// the member's handle/id. Deactivation revokes the member's tokens.
#[derive(Debug, Clone)]
pub struct ScimUser {
    pub member_id: MemberId,
    pub workspace_id: WorkspaceId,
    pub external_id: Option<String>,
    pub active: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A SCIM 2.0 Group (RFC 7643 §4.2): an identity provider's named set of the
/// users it provisioned into one workspace. It records who the IdP grouped
/// together and grants nothing by itself.
#[derive(Debug, Clone)]
pub struct ScimGroup {
    pub id: ScimGroupId,
    pub workspace_id: WorkspaceId,
    pub display_name: String,
    pub external_id: Option<String>,
    /// Ordered by handle.
    pub members: Vec<ScimGroupMember>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A group member, with the handle read at query time so a rename shows at
/// once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScimGroupMember {
    pub member_id: MemberId,
    pub handle: String,
}

/// A SCIM group to create. Every member must be a SCIM user of the workspace.
#[derive(Debug, Clone)]
pub struct NewScimGroup {
    pub workspace_id: WorkspaceId,
    pub display_name: String,
    pub external_id: Option<String>,
    pub members: Vec<MemberId>,
}

/// A change to a SCIM group: a PUT sets every field, a PATCH the ones it
/// names. Membership operations apply in order, as a PatchOp's do.
#[derive(Debug, Clone, Default)]
pub struct ScimGroupChange {
    pub display_name: Option<String>,
    /// `Some(None)` clears the external id.
    pub external_id: Option<Option<String>>,
    pub members: Vec<ScimMembersOp>,
}

/// One membership operation of a [`ScimGroupChange`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScimMembersOp {
    Add(Vec<MemberId>),
    Remove(Vec<MemberId>),
    /// Make the member set exactly this; empty removes everyone.
    Replace(Vec<MemberId>),
}

/// A committed group change: the group as it now is, and the net membership
/// difference, which is what its audit row records.
#[derive(Debug, Clone)]
pub struct ScimGroupWrite {
    pub group: ScimGroup,
    pub added: Vec<MemberId>,
    pub removed: Vec<MemberId>,
}

/// A member due for an email digest: the sweeper's enumeration row — a
/// digest-mode member with an address who has unread notifications created
/// since their last digest. Carries the address so the sweeper needs no extra
/// per-member lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigestDue {
    pub member_id: MemberId,
    pub email: String,
    pub unread_count: i64,
    /// The member's digest watermark — decisions produced after this instant
    /// are the "buried" ones the digest surfaces. `None` = never digested
    /// (treat as the epoch).
    pub last_digest_at: Option<DateTime<Utc>>,
}

/// A decision the member may have missed — a task result produced by someone
/// else in a channel or thread the member follows, since their last digest. The
/// buried-decisions digest lists these instead of a bare unread count; it's
/// also queryable directly.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct BuriedDecision {
    pub thread_id: ThreadId,
    pub channel_id: ChannelId,
    pub thread_title: Option<String>,
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub result: serde_json::Value,
    pub produced_by: MemberId,
    pub produced_at: DateTime<Utc>,
}

/// Notification-backed management rollup for one channel. `None` is the
/// workspace-level bucket (currently unattached approval gates).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ManagerDigestChannel {
    pub channel_id: Option<ChannelId>,
    pub results: i64,
    pub gates: i64,
    pub stuck: i64,
}

/// A member's unread followed-member lifecycle notifications since `since`,
/// composed into per-channel result/gate/stuck counts. This is a notification
/// view, not an analytics projection.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ManagerDigest {
    pub member_id: MemberId,
    pub since: DateTime<Utc>,
    pub channels: Vec<ManagerDigestChannel>,
    /// Lifetime workspace spend in micro-USD. Not limited to `since`.
    #[serde(default)]
    pub spend_usd_micros: i64,
    /// Lifetime cost per completed task. Absent when the workspace has none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cost_per_completed_task_usd_micros: Option<i64>,
}

/// System channel name for DM threads in a workspace.
pub const DM_CHANNEL_NAME: &str = "__dm__";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DmConversation {
    pub id: DmConversationId,
    pub workspace_id: WorkspaceId,
    pub member_low_id: MemberId,
    pub member_high_id: MemberId,
    pub thread_id: ThreadId,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct OpenDmConversation {
    pub other_member_id: uuid::Uuid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct GroupDmConversation {
    pub id: GroupDmConversationId,
    pub workspace_id: WorkspaceId,
    pub thread_id: ThreadId,
    pub title: Option<String>,
    pub member_ids: Vec<MemberId>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct OpenGroupDmBody {
    pub member_ids: Vec<uuid::Uuid>,
    pub title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PostDmMessage {
    pub body: String,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Thread {
    pub id: ThreadId,
    pub channel_id: ChannelId,
    pub parent_thread_id: Option<ThreadId>,
    pub title: Option<String>,
    /// What the task is about, in the creator's words. Set at creation;
    /// `None` on threads created before descriptions existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub state: ThreadState,
    /// The member this thread/task is assigned to, if any. An axis orthogonal
    /// to [`ThreadState`]: assignment persists across state transitions. Set
    /// via assign/handoff, atomic claim, or cleared on unassign.
    pub assignee_id: Option<MemberId>,
    /// Lease deadline for a claimed assignment. When set and in the past, the
    /// assignment is reclaimable by the next `claim_next` (dead-agent
    /// recovery); `None` is a durable assignment with no lease.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignment_expires_at: Option<DateTime<Utc>>,
    /// The fencing value for the current claim. A fresh resource-version minted
    /// every time `assignee_id` is set (claim / claim_next / assign) and
    /// cleared on unassign. `renew_claim` and other claim-holder operations
    /// must present the matching value — a TTL lease alone lets a stale holder
    /// act after the next owner has taken over.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claim_lease_id: Option<ClaimLeaseId>,
    /// The working clock. `assignment_expires_at` is the *claim* clock (lease
    /// deadline); this is when the current holder acknowledged and began work
    /// (`acknowledge_claim`). `None` = claimed but not yet started, or
    /// unassigned. Reset to `None` on every (re)claim/assign/unassign so it
    /// always reflects the CURRENT claim epoch — letting occupancy separate a
    /// claimed-but-idle agent from one actively working.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_started_at: Option<DateTime<Utc>>,
    /// The durable OWNER of this task/thread: the accountable party — a human,
    /// typically — distinct from the [`Thread::assignee_id`] claimer that does
    /// the work. Orthogonal to the FSM and the claim axis. The owner receives
    /// stuck notifications and, once set, opts the thread into
    /// separation-of-duties (the claimer cannot land its own work). `None` = no
    /// designated owner (unrestricted).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_id: Option<MemberId>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub tombstoned_at: Option<DateTime<Utc>>,
    /// The agent's self-reported status, if declared. Set via `declare_status`;
    /// cleared on human response. `None` = no active declaration.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<ThreadStatusDeclaration>,
    /// The thread's explicit dispatch block, if any. Populated by the API
    /// layer when returning thread details; `None` in store-level queries
    /// that do not JOIN the blocks table.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block: Option<ThreadBlock>,
    /// The thread is closed (or archived) and no approval stands on it. The
    /// review gate is opt-in, so a close can need none; the board says so
    /// rather than showing a plain "done". Read by a single-thread read and a
    /// channel's thread page, the reads a board makes; `false` on the other
    /// list reads.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub closed_without_review: bool,
}

#[derive(Debug, Clone)]
pub struct NewThread {
    pub channel_id: ChannelId,
    pub parent_thread_id: Option<ThreadId>,
    pub title: Option<String>,
    pub description: Option<String>,
}

/// A child thread collapsed under its parent: the child thread plus a live
/// count of its (non-tombstoned) messages, so a threaded view can show "N
/// replies" without loading each child's messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ChildThreadSummary {
    pub thread: Thread,
    pub message_count: i64,
}

#[derive(Debug, Clone)]
pub struct ThreadTransitionResult {
    pub thread: Thread,
    pub from_state: ThreadState,
    pub to_state: ThreadState,
}

/// Outcome of an atomic [`Thread`] claim: `claimed` is `true` when this call
/// won the compare-and-set (the thread was unassigned and is now the caller's),
/// `false` when it was already assigned. `thread` is the current row either
/// way.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ThreadClaimResult {
    pub thread: Thread,
    pub claimed: bool,
}

/// A channel's task-queue depth — a point-in-time partition of its
/// **open** (non-terminal, non-tombstoned) task threads, for an orchestrator
/// deciding whether to scale workers. The three sub-counts partition `open`:
/// - `assigned`: actively held (an assignee with a live, non-expired lease).
/// - `ready`: claimable now — unassigned or lease-expired, and every dependency
///   terminal (the `claim_next` predicate).
/// - `blocked`: unassigned/lease-expired but waiting on a non-terminal dependency.
/// - `unclaimable`: unassigned/lease-expired but parked from dispatch —
///   `claim_next` skips it. Takes precedence over ready/blocked, so the four
///   sub-counts partition `open` exactly.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct QueueDepth {
    pub open: i64,
    pub ready: i64,
    pub assigned: i64,
    pub blocked: i64,
    pub unclaimable: i64,
}

/// The occupancy of a channel's **open** task threads — the two-clocks
/// refinement of [`QueueDepth`]. It splits `assigned` by the *working* clock,
/// so an orchestrator sees not just how much work is held but how much is
/// actually underway. The four sub-counts partition `open`:
/// - `queued`: claimable now — unassigned or lease-expired, with every dependency
///   terminal (the `QueueDepth::ready` predicate).
/// - `claimed`: held on a live lease, but the holder has not yet acknowledged —
///   `work_started_at` is unset (grabbed the work, hasn't started).
/// - `working`: held and the holder has acknowledged and begun work
///   (`work_started_at` is set).
/// - `blocked`: not held and waiting on a non-terminal dependency.
///
/// Splitting `claimed` from `working` is the payoff of the two clocks: it
/// surfaces a claimed-but-idle agent — one that took work but never started it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ChannelOccupancy {
    pub open: i64,
    pub queued: i64,
    pub claimed: i64,
    pub working: i64,
    pub blocked: i64,
}

/// A schedule that materializes a task thread when due. A one-shot
/// (`interval_secs == None`) fires once then deactivates; a recurring schedule
/// (`interval_secs == Some(n)`) re-arms `next_run_at += n s` after each firing.
/// The background sweeper (a later cluster) creates a thread titled `title` in
/// `channel_id` when `active && next_run_at <= now`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TaskSchedule {
    pub id: TaskScheduleId,
    pub workspace_id: WorkspaceId,
    pub channel_id: ChannelId,
    pub title: String,
    pub interval_secs: Option<i64>,
    pub next_run_at: DateTime<Utc>,
    pub last_run_at: Option<DateTime<Utc>>,
    pub active: bool,
    pub created_by: MemberId,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// When set, firing this schedule instantiates the recipe (parent + DAG
    /// children, copy-on-fire) instead of creating one bare thread.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recipe_id: Option<RecipeId>,
}

#[derive(Debug, Clone)]
pub struct NewTaskSchedule {
    pub workspace_id: WorkspaceId,
    pub channel_id: ChannelId,
    pub title: String,
    pub interval_secs: Option<i64>,
    pub next_run_at: DateTime<Utc>,
    pub created_by: MemberId,
    pub recipe_id: Option<RecipeId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ThreadTransition {
    pub id: uuid::Uuid,
    pub thread_id: ThreadId,
    pub from_state: ThreadState,
    pub to_state: ThreadState,
    pub actor_id: MemberId,
    pub occurred_at: DateTime<Utc>,
}

/// A task-dependency DAG edge: the task `thread_id` depends on
/// `depends_on_thread_id` — i.e. it is blocked until that dependency reaches a
/// terminal state. Edges are directed; the pair is unique.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ThreadDependency {
    pub thread_id: ThreadId,
    pub depends_on_thread_id: ThreadId,
    pub created_at: DateTime<Utc>,
}

/// A typed part of a message's structured content. The wire form is internally
/// tagged (`{"type":"text","text":"…"}`), matching the MCP / Anthropic
/// content-block dialect and the existing A2A `TextPart`. `body` remains the
/// canonical searchable plain-text projection derived from these.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    Code {
        #[serde(skip_serializing_if = "Option::is_none", default)]
        language: Option<String>,
        code: String,
    },
    /// A tool/function invocation (agent → tool).
    ToolUse {
        id: String,
        name: String,
        input: serde_json::Value,
    },
    /// The result of a prior [`ContentBlock::ToolUse`], correlated by id.
    ToolResult {
        tool_use_id: String,
        content: String,
        #[serde(default)]
        is_error: bool,
    },
    /// A pointer to a resource/artifact (URI form).
    ResourceLink {
        uri: String,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        mime_type: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none", default)]
        title: Option<String>,
    },
}

/// Derive the plain-text `body` projection from structured content blocks so
/// full-text + semantic search stay unchanged. `ToolUse` adds nothing (a tool
/// name is not prose); code is fenced; a resource link renders as its title or
/// URI. Blocks are joined by blank lines.
pub fn derive_body(blocks: &[ContentBlock]) -> String {
    blocks
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text } => Some(text.clone()),
            ContentBlock::Code { language, code } => Some(format!(
                "```{}\n{code}\n```",
                language.as_deref().unwrap_or("")
            )),
            ContentBlock::ToolResult { content, .. } => Some(content.clone()),
            ContentBlock::ResourceLink { uri, title, .. } => {
                Some(title.clone().unwrap_or_else(|| uri.clone()))
            }
            ContentBlock::ToolUse { .. } => None,
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// Content-addressed artifact SHAs referenced by a message's `metadata` — the
/// `artifact_sha256` / `sha256` scalar fields plus an `artifacts` array of
/// either bare SHA strings or `{sha256}` objects. Sorted + deduped. Shared by
/// the REST and MCP context assemblers so both surface the same artifacts.
pub fn artifact_shas_from_metadata(metadata: &serde_json::Value) -> Vec<String> {
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

/// One tool invocation in a thread's transcript: a [`ContentBlock::ToolUse`]
/// paired with its [`ContentBlock::ToolResult`] (correlated by id), plus the
/// message context each block came from.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ToolCallEntry {
    pub tool_use_id: String,
    pub name: String,
    pub input: serde_json::Value,
    pub message_id: MessageId,
    pub author_id: MemberId,
    pub posted_at: DateTime<Utc>,
    /// The correlated result, if a matching `ToolResult` was found.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub result: Option<ToolCallResult>,
}

/// The result side of a [`ToolCallEntry`], from the message carrying the
/// matching `ToolResult` block.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ToolCallResult {
    pub content: String,
    pub is_error: bool,
    pub message_id: MessageId,
    pub author_id: MemberId,
    pub posted_at: DateTime<Utc>,
}

/// A `ToolResult` block whose `tool_use_id` matched no `ToolUse` in the scanned
/// messages — surfaced rather than dropped so a gap is visible.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct OrphanToolResult {
    pub tool_use_id: String,
    pub content: String,
    pub is_error: bool,
    pub message_id: MessageId,
    pub author_id: MemberId,
    pub posted_at: DateTime<Utc>,
}

/// A thread's tool-call transcript: every [`ContentBlock::ToolUse`] across the
/// thread's messages, each correlated with its `ToolResult` by id, plus any
/// results whose call is outside the scanned window. A token-lean projection of
/// the tool structure — `Text`/`Code`/`ResourceLink` blocks and `body` are
/// dropped.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ToolTranscript {
    pub thread_id: ThreadId,
    pub entries: Vec<ToolCallEntry>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub orphan_results: Vec<OrphanToolResult>,
}

/// Extract a [`ToolTranscript`] from a thread's messages. Walks each
/// non-tombstoned message's structured content, pairing every `ToolUse` with
/// the first `ToolResult` carrying the same id (correlation is
/// order-independent — a result may land in a later message). A result with no
/// matching call is an orphan; a duplicate result for an already-resolved call
/// is treated as an orphan too. `messages` should be chronological; entry order
/// follows the calls' order.
pub fn tool_transcript(thread_id: ThreadId, messages: &[Message]) -> ToolTranscript {
    use std::collections::HashMap;
    let mut entries: Vec<ToolCallEntry> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    let mut orphan_results: Vec<OrphanToolResult> = Vec::new();

    let live = || messages.iter().filter(|m| m.tombstoned_at.is_none());

    for m in live() {
        let Some(blocks) = m.content.as_ref() else {
            continue;
        };
        for block in blocks {
            if let ContentBlock::ToolUse { id, name, input } = block {
                // A duplicate id keeps the first call; later ones aren't
                // distinguishable for correlation.
                if !index.contains_key(id) {
                    index.insert(id.clone(), entries.len());
                    entries.push(ToolCallEntry {
                        tool_use_id: id.clone(),
                        name: name.clone(),
                        input: input.clone(),
                        message_id: m.id,
                        author_id: m.author_id,
                        posted_at: m.posted_at,
                        result: None,
                    });
                }
            }
        }
    }

    for m in live() {
        let Some(blocks) = m.content.as_ref() else {
            continue;
        };
        for block in blocks {
            if let ContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error,
            } = block
            {
                match index.get(tool_use_id) {
                    Some(&i) if entries[i].result.is_none() => {
                        entries[i].result = Some(ToolCallResult {
                            content: content.clone(),
                            is_error: *is_error,
                            message_id: m.id,
                            author_id: m.author_id,
                            posted_at: m.posted_at,
                        });
                    }
                    _ => orphan_results.push(OrphanToolResult {
                        tool_use_id: tool_use_id.clone(),
                        content: content.clone(),
                        is_error: *is_error,
                        message_id: m.id,
                        author_id: m.author_id,
                        posted_at: m.posted_at,
                    }),
                }
            }
        }
    }

    ToolTranscript {
        thread_id,
        entries,
        orphan_results,
    }
}

/// `true` for a JSON value that carries no information — `null` or an empty
/// object — used to omit an empty `metadata` from the wire.
fn json_value_is_empty(v: &serde_json::Value) -> bool {
    v.is_null() || v.as_object().is_some_and(|o| o.is_empty())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Message {
    pub id: MessageId,
    pub thread_id: ThreadId,
    pub author_id: MemberId,
    pub body: String,
    /// Open annotation bag. Omitted from the wire when empty — most messages
    /// carry no metadata, so `"metadata":{}` on every one was pure token waste.
    /// Deserializes back to an empty object by default.
    #[serde(skip_serializing_if = "json_value_is_empty", default)]
    pub metadata: serde_json::Value,
    /// Typed structured content; `None` for plain/legacy messages. `body` is
    /// the plain-text projection of these blocks.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub content: Option<Vec<ContentBlock>>,
    pub posted_at: DateTime<Utc>,
    pub edited_at: Option<DateTime<Utc>>,
    pub tombstoned_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct NewMessage {
    pub thread_id: ThreadId,
    pub author_id: MemberId,
    pub body: String,
    pub metadata: serde_json::Value,
    pub content: Option<Vec<ContentBlock>>,
}

/// Body/metadata replacement for [`Store::edit_message`].
#[derive(Debug, Clone)]
pub struct EditMessage {
    pub body: String,
    pub metadata: serde_json::Value,
    pub content: Option<Vec<ContentBlock>>,
}

/// One recorded body change for a message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MessageEdit {
    pub id: i64,
    pub message_id: MessageId,
    pub editor_id: MemberId,
    pub body_before: String,
    pub body_after: String,
    pub edited_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Mention {
    pub message_id: MessageId,
    pub member_id: MemberId,
    pub created_at: DateTime<Utc>,
}

/// What is waiting on a member: the class of a [`WaitingItem`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum WaitingKind {
    /// A non-terminal thread assigned to the member.
    AssignedThread,
    /// A pending approval gate needing a human.
    OpenGate,
    /// An unread @mention of the member.
    Mention,
    /// A thread under review that names the member as a reviewer and does not
    /// have their approval yet.
    ReviewRequest,
    /// A thread blocked with reason `human` or `gate`, waiting on its owner
    /// (or workspace admins when it has no owner) to unblock it.
    Blocked,
    /// A thread under review that names no reviewer, so no review request
    /// reaches anyone. It waits on the thread's owner, or, when it has none,
    /// on the workspace's admins.
    UnassignedReview,
    /// An agent declared `needs_input`: it asked a question and waits for a
    /// human to answer in the thread. It reaches the thread's owner, or the
    /// workspace's admins when the thread has no owner or the owner asked.
    Question,
}

/// One thing waiting on a member — with its age and whether it has breached the
/// SLA.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WaitingItem {
    pub kind: WaitingKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<ThreadId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate_id: Option<ApprovalGateId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<MessageId>,
    pub summary: String,
    /// The part of `summary` a reader acts on, when it has one: an agent's
    /// question, which `summary` puts after the thread's title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub since: DateTime<Utc>,
    pub age_secs: i64,
    pub overdue: bool,
}

/// The waiting-on-you inbox: everything that needs a member's attention —
/// assigned tasks, open gates, unread mentions — oldest-waiting first, with an
/// SLA marking the overdue ones. Not `@everyone`: it is one member's queue.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WaitingInbox {
    pub items: Vec<WaitingItem>,
    pub total: usize,
    pub overdue: usize,
    pub sla_secs: i64,
}

fn truncate_summary(s: &str, max_chars: usize) -> String {
    let trimmed = s.trim();
    if trimmed.chars().count() <= max_chars {
        trimmed.to_string()
    } else {
        let head: String = trimmed.chars().take(max_chars).collect();
        format!("{head}…")
    }
}

fn waiting_item(
    kind: WaitingKind,
    thread_id: Option<ThreadId>,
    gate_id: Option<ApprovalGateId>,
    message_id: Option<MessageId>,
    summary: String,
    since: DateTime<Utc>,
    now: DateTime<Utc>,
    sla_secs: i64,
) -> WaitingItem {
    let age_secs = (now - since).num_seconds().max(0);
    WaitingItem {
        kind,
        thread_id,
        gate_id,
        message_id,
        summary,
        detail: None,
        since,
        age_secs,
        overdue: age_secs > sla_secs,
    }
}

/// What waits on one member, already fetched and filtered by the caller for
/// access, read state and routing. An omitted source is empty.
#[derive(Debug, Default)]
pub struct WaitingSources<'a> {
    /// Their assigned threads; terminal ones are skipped.
    pub assigned: &'a [Thread],
    /// The reviews requested from them.
    pub review_requests: &'a [Thread],
    /// The reviews that name no reviewer and fall to them.
    pub unassigned_reviews: &'a [Thread],
    /// The workspace's pending approval gates (they need a human).
    pub pending_gates: &'a [ApprovalGate],
    /// Their unread mentions.
    pub unread_mentions: &'a [Mention],
    /// Threads blocked on a human or a gate that reach them.
    pub blocked: &'a [(ThreadId, Option<String>, Option<MemberId>, ThreadBlock)],
    /// Agents' questions (`needs_input`) that reach them.
    pub questions: &'a [(
        ThreadId,
        Option<String>,
        Option<MemberId>,
        ThreadStatusDeclaration,
    )],
}

/// Assemble a member's waiting-on-you inbox from its sources. Pure: the
/// caller fetches the sources and filters them; items come back
/// oldest-waiting first, each aged against `sla_secs`.
pub fn assemble_waiting_inbox(
    sources: &WaitingSources<'_>,
    now: DateTime<Utc>,
    sla_secs: i64,
) -> WaitingInbox {
    let WaitingSources {
        assigned,
        review_requests,
        unassigned_reviews,
        pending_gates,
        unread_mentions,
        blocked,
        questions,
    } = *sources;
    let mut items = Vec::new();
    for t in assigned {
        if t.state.is_terminal() || t.tombstoned_at.is_some() {
            continue;
        }
        items.push(waiting_item(
            WaitingKind::AssignedThread,
            Some(t.id),
            None,
            None,
            t.title
                .clone()
                .unwrap_or_else(|| "(untitled thread)".to_string()),
            t.created_at,
            now,
            sla_secs,
        ));
    }
    for t in review_requests {
        if t.state.is_terminal() || t.tombstoned_at.is_some() {
            continue;
        }
        items.push(waiting_item(
            WaitingKind::ReviewRequest,
            Some(t.id),
            None,
            None,
            t.title
                .clone()
                .unwrap_or_else(|| "(untitled thread)".to_string()),
            t.updated_at,
            now,
            sla_secs,
        ));
    }
    for t in unassigned_reviews {
        if t.state.is_terminal() || t.tombstoned_at.is_some() {
            continue;
        }
        items.push(waiting_item(
            WaitingKind::UnassignedReview,
            Some(t.id),
            None,
            None,
            t.title
                .clone()
                .unwrap_or_else(|| "(untitled thread)".to_string()),
            t.updated_at,
            now,
            sla_secs,
        ));
    }
    for g in pending_gates {
        items.push(waiting_item(
            WaitingKind::OpenGate,
            g.thread_id,
            Some(g.id),
            None,
            truncate_summary(&g.prompt, 120),
            g.created_at,
            now,
            sla_secs,
        ));
    }
    for m in unread_mentions {
        items.push(waiting_item(
            WaitingKind::Mention,
            None,
            None,
            Some(m.message_id),
            format!("mention in message {}", m.message_id.0),
            m.created_at,
            now,
            sla_secs,
        ));
    }
    for (tid, title, _owner, block) in blocked {
        let summary = match &block.note {
            Some(n) if !n.is_empty() => format!("blocked ({}): {}", block.reason.as_str(), n),
            _ => format!("blocked: {}", block.reason.as_str()),
        };
        items.push(waiting_item(
            WaitingKind::Blocked,
            Some(*tid),
            None,
            None,
            title
                .clone()
                .unwrap_or_else(|| "(untitled thread)".to_string())
                + " — "
                + &summary,
            block.set_at,
            now,
            sla_secs,
        ));
    }
    for (tid, title, _owner, declaration) in questions {
        let mut item = waiting_item(
            WaitingKind::Question,
            Some(*tid),
            None,
            None,
            title
                .clone()
                .unwrap_or_else(|| "(untitled thread)".to_string())
                + ": "
                + &declaration.note,
            declaration.declared_at,
            now,
            sla_secs,
        );
        item.detail = Some(declaration.note.clone());
        items.push(item);
    }
    items.sort_by(|a, b| a.since.cmp(&b.since));
    let overdue = items.iter().filter(|i| i.overdue).count();
    WaitingInbox {
        total: items.len(),
        overdue,
        sla_secs,
        items,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum InboxItemKind {
    Mention,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct InboxItem {
    pub kind: InboxItemKind,
    pub message_id: MessageId,
    pub member_id: MemberId,
    pub created_at: DateTime<Utc>,
    pub unread: bool,
    pub message_body: String,
    pub thread_id: ThreadId,
    pub channel_id: ChannelId,
    pub author_id: MemberId,
    pub author_handle: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MemberInbox {
    pub items: Vec<InboxItem>,
    pub unread_count: i64,
    pub last_read_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MarkInboxRead {
    pub read_through: DateTime<Utc>,
}

/// The only vote kinds the server stores. An emoji is a reaction, not a kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum VoteKind {
    Approve,
    RequestChanges,
    Ack,
}

impl VoteKind {
    pub const ALL: [Self; 3] = [Self::Approve, Self::RequestChanges, Self::Ack];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Approve => "approve",
            Self::RequestChanges => "request_changes",
            Self::Ack => "ack",
        }
    }

    /// `None` for every string outside [`ALL`](Self::ALL), including `up`,
    /// `upvote`, `request-changes`, and an emoji.
    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == s)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Vote {
    pub message_id: MessageId,
    pub member_id: MemberId,
    pub kind: VoteKind,
    /// Optional confidence weight, by convention in `0..=1`, for weighted
    /// consensus. `None` when the voter stated no confidence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewVote {
    pub message_id: MessageId,
    pub member_id: MemberId,
    pub kind: VoteKind,
    /// Optional confidence weight, by convention `0..=1`.
    pub confidence: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Reaction {
    pub message_id: MessageId,
    pub member_id: MemberId,
    pub emoji: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewReaction {
    pub message_id: MessageId,
    pub member_id: MemberId,
    pub emoji: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Pin {
    pub thread_id: ThreadId,
    pub message_id: MessageId,
    pub member_id: MemberId,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewPin {
    pub thread_id: ThreadId,
    pub message_id: MessageId,
    pub member_id: MemberId,
}

/// The typed predicate on a [`Reference`] edge. A small controlled vocabulary —
/// the same subject→predicate→object shape as IBIS, W3C PROV, ClaimReview, and
/// GitHub/Linear issue relations — so an agent's edges are machine-navigable
/// ("what `refutes` this", "what this `supersedes`") instead of free prose.
/// [`Other`] keeps expressivity: an unrecognized relation round-trips verbatim
/// rather than being rejected. Serializes as the bare snake_case string on the
/// wire (a controlled variant → its canonical name; `Other(s)` → `s`).
///
/// [`Other`]: RelationKind::Other
/// On the wire a relation is its name, a controlled one or any other string.
#[cfg(feature = "openapi")]
impl utoipa::PartialSchema for RelationKind {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        utoipa::openapi::ObjectBuilder::new()
            .schema_type(utoipa::openapi::schema::Type::String)
            .description(Some(
                "A relation name: supports, refutes, defines, depends, duplicates, grounds, supersedes, seeded_from, or any other string",
            ))
            .into()
    }
}

#[cfg(feature = "openapi")]
impl utoipa::ToSchema for RelationKind {
    fn name() -> std::borrow::Cow<'static, str> {
        "RelationKind".into()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RelationKind {
    /// This entity provides support/evidence for the target.
    Supports,
    /// This entity contradicts/refutes the target.
    Refutes,
    /// This entity defines the target (points at a glossary term / canonical def).
    Defines,
    /// This entity depends on the target.
    Depends,
    /// This entity is a duplicate of the target.
    Duplicates,
    /// This entity is grounded in the target (source span / artifact / provenance).
    Grounds,
    /// This entity supersedes the target (the target is now historical).
    Supersedes,
    /// This entity was seeded/branched from the target (re-ask lineage): a new
    /// work thread spawned from a source message.
    SeededFrom,
    /// Any relation outside the controlled set, preserved verbatim.
    Other(String),
}

impl RelationKind {
    /// The controlled vocabulary (excludes `Other`).
    pub const CONTROLLED: [&'static str; 8] = [
        "supports",
        "refutes",
        "defines",
        "depends",
        "duplicates",
        "grounds",
        "supersedes",
        "seeded_from",
    ];

    /// The wire string for this relation.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Supports => "supports",
            Self::Refutes => "refutes",
            Self::Defines => "defines",
            Self::Depends => "depends",
            Self::Duplicates => "duplicates",
            Self::Grounds => "grounds",
            Self::Supersedes => "supersedes",
            Self::SeededFrom => "seeded_from",
            Self::Other(s) => s,
        }
    }

    /// Parse a wire string into a relation — a controlled variant when it matches,
    /// otherwise `Other` (never fails).
    pub fn from_wire(s: &str) -> Self {
        match s {
            "supports" => Self::Supports,
            "refutes" => Self::Refutes,
            "defines" => Self::Defines,
            "depends" => Self::Depends,
            "duplicates" => Self::Duplicates,
            "grounds" => Self::Grounds,
            "supersedes" => Self::Supersedes,
            "seeded_from" => Self::SeededFrom,
            other => Self::Other(other.to_string()),
        }
    }

    /// True for a controlled-vocabulary relation (not `Other`).
    pub fn is_controlled(&self) -> bool {
        !matches!(self, Self::Other(_))
    }
}

impl From<&str> for RelationKind {
    fn from(s: &str) -> Self {
        Self::from_wire(s)
    }
}

impl From<String> for RelationKind {
    fn from(s: String) -> Self {
        Self::from_wire(&s)
    }
}

impl Serialize for RelationKind {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for RelationKind {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        Ok(Self::from_wire(&s))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Reference {
    pub id: uuid::Uuid,
    pub src_kind: RefSide,
    pub src_id: uuid::Uuid,
    pub dst_kind: RefSide,
    pub dst_id: uuid::Uuid,
    /// The typed predicate. Wire form is a snake_case string; the controlled
    /// set is [`RelationKind::CONTROLLED`], unknown values round-trip via
    /// [`RelationKind::Other`].
    #[cfg_attr(feature = "openapi", schema(value_type = String))]
    pub relation: RelationKind,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewReference {
    pub src_kind: RefSide,
    pub src_id: uuid::Uuid,
    pub dst_kind: RefSide,
    pub dst_id: uuid::Uuid,
    pub relation: RelationKind,
}

/// A workspace's canonical definition of a term — the anti-drift pin so agents
/// use words the same way, and the target of the `defines` reference relation.
/// One entry per `(workspace_id, term)`. Flat by design (no hierarchy).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct GlossaryTerm {
    pub id: uuid::Uuid,
    pub workspace_id: WorkspaceId,
    pub term: String,
    pub definition: String,
    /// Alternate labels for the same term (SKOS altLabel).
    pub aliases: Vec<String>,
    pub created_by: MemberId,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewGlossaryTerm {
    pub workspace_id: WorkspaceId,
    pub term: String,
    pub definition: String,
    pub aliases: Vec<String>,
    pub created_by: MemberId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Artifact {
    pub id: ArtifactId,
    pub sha256: String,
    pub size_bytes: i64,
    pub mime_type: Option<String>,
    /// The name the caller's workspace uploaded the bytes under, for display
    /// only: it never locates anything. Another workspace holding the same
    /// bytes sees its own name, or none. Omitted when absent, so an event
    /// written before the field existed serializes as it was written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    pub kind: ArtifactKind,
    pub uploaded_by: Option<MemberId>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewArtifact {
    pub sha256: String,
    pub size_bytes: i64,
    pub mime_type: Option<String>,
    pub filename: Option<String>,
    pub kind: ArtifactKind,
    pub uploaded_by: Option<MemberId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AuditEvent {
    pub id: i64,
    pub occurred_at: DateTime<Utc>,
    /// Who performed the action. For a delegated action, the delegate.
    pub actor_id: Option<MemberId>,
    /// Who the action was performed for — the actor itself unless delegated.
    #[serde(default)]
    pub subject_id: Option<MemberId>,
    /// The delegation grant the action was taken under, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grant_id: Option<DelegationGrantId>,
    pub action: String,
    pub target_kind: Option<String>,
    pub target_id: Option<uuid::Uuid>,
    pub metadata: serde_json::Value,
    /// The workspace the row belongs to: the one whose audit view shows it and
    /// whose legal hold keeps it. `None` is an instance-level row, shown only in
    /// the operator audit and pruned under instance policy.
    #[serde(default)]
    pub workspace_id: Option<WorkspaceId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct App {
    pub id: AppId,
    pub workspace_id: WorkspaceId,
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub created_by: MemberId,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewApp {
    pub workspace_id: WorkspaceId,
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub created_by: MemberId,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppInstallation {
    pub id: AppInstallationId,
    pub app_id: AppId,
    pub workspace_id: WorkspaceId,
    pub bot_member_id: MemberId,
    pub granted_capabilities: Vec<String>,
    pub installed_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct NewAppInstallation {
    pub app_id: AppId,
    pub workspace_id: WorkspaceId,
    pub bot_member_id: MemberId,
    pub granted_capabilities: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiToken {
    pub id: ApiTokenId,
    pub workspace_id: WorkspaceId,
    pub member_id: MemberId,
    pub app_installation_id: Option<AppInstallationId>,
    pub token_hash: String,
    pub label: Option<String>,
    pub capabilities: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    /// Present for a grant-exchanged token and every attenuated descendant.
    pub delegation_grant_id: Option<DelegationGrantId>,
}

#[derive(Debug, Clone)]
pub struct NewApiToken {
    pub workspace_id: WorkspaceId,
    pub member_id: MemberId,
    pub app_installation_id: Option<AppInstallationId>,
    pub token_hash: String,
    pub label: Option<String>,
    pub capabilities: Vec<String>,
    pub expires_at: Option<DateTime<Utc>>,
}

/// A one-time OAuth authorization code persisted for cross-replica exchange.
/// Only the SHA-256 hash of the code is stored.
#[derive(Debug, Clone)]
pub struct NewOAuthCode {
    pub code_hash: String,
    pub app_id: AppId,
    pub workspace_id: WorkspaceId,
    pub redirect_uri: String,
    pub code_challenge: Option<String>,
    pub expires_at: DateTime<Utc>,
}

/// A consumed OAuth authorization code's payload (see [`NewOAuthCode`]).
#[derive(Debug, Clone)]
pub struct OAuthCode {
    pub app_id: AppId,
    pub workspace_id: WorkspaceId,
    pub redirect_uri: String,
    pub code_challenge: Option<String>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TokenQuota {
    pub capability: String,
    pub max_per_window: u32,
    pub window_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Peer {
    pub id: PeerId,
    pub workspace_id: WorkspaceId,
    pub remote_workspace_id: WorkspaceId,
    pub name: String,
    pub base_url: String,
    #[serde(skip_serializing)]
    pub token_hash: String,
    #[serde(skip_serializing)]
    pub outbound_secret_ciphertext: Option<String>,
    pub enabled: bool,
    pub last_synced_event_id: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewPeer {
    pub workspace_id: WorkspaceId,
    pub remote_workspace_id: WorkspaceId,
    pub name: String,
    pub base_url: String,
    pub token_hash: String,
    pub outbound_secret_ciphertext: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct OidcIdentity {
    pub id: OidcIdentityId,
    pub workspace_id: WorkspaceId,
    pub issuer: String,
    pub subject: String,
    pub member_id: MemberId,
    pub email: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_login_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MaidanSession {
    pub id: SessionId,
    pub workspace_id: WorkspaceId,
    pub member_id: MemberId,
    /// The token a `POST /auth/session/from-token` session was made from. Each
    /// request re-resolves it, so the session holds that token's authority and
    /// ends with it. `None` for an OIDC session.
    pub api_token_id: Option<ApiTokenId>,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewMaidanSession {
    pub workspace_id: WorkspaceId,
    pub member_id: MemberId,
    pub api_token_id: Option<ApiTokenId>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewOidcIdentity {
    pub workspace_id: WorkspaceId,
    pub issuer: String,
    pub subject: String,
    pub member_id: MemberId,
    pub email: Option<String>,
}

#[derive(Debug, Clone)]
pub struct OidcPendingAuth {
    pub state: String,
    pub workspace_id: WorkspaceId,
    pub nonce: String,
    pub pkce_verifier: String,
    pub return_to: Option<String>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewOidcPendingAuth {
    pub state: String,
    pub workspace_id: WorkspaceId,
    pub nonce: String,
    pub pkce_verifier: String,
    pub return_to: Option<String>,
    pub expires_at: DateTime<Utc>,
}

/// Which workspace an audit row belongs to. It has no default on purpose: the
/// row's workspace decides who sees it and which legal hold keeps it, so every
/// writer states it rather than inheriting a guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuditScope {
    Workspace(WorkspaceId),
    /// Not any one workspace's: an instance-wide operation, or one whose
    /// workspace is not known when it is recorded.
    Instance,
}

impl AuditScope {
    pub fn workspace_id(self) -> Option<WorkspaceId> {
        match self {
            AuditScope::Workspace(ws) => Some(ws),
            AuditScope::Instance => None,
        }
    }
}

impl From<WorkspaceId> for AuditScope {
    fn from(ws: WorkspaceId) -> Self {
        AuditScope::Workspace(ws)
    }
}

#[derive(Debug, Clone)]
pub struct NewAuditEvent {
    pub scope: AuditScope,
    pub actor_id: Option<MemberId>,
    pub action: String,
    pub target_kind: Option<String>,
    pub target_id: Option<uuid::Uuid>,
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WebhookSubscription {
    pub id: WebhookSubscriptionId,
    pub workspace_id: WorkspaceId,
    pub url: String,
    pub label: Option<String>,
    pub event_kinds: Vec<String>,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct NewWebhookSubscription {
    pub workspace_id: WorkspaceId,
    pub url: String,
    pub label: Option<String>,
    pub event_kinds: Vec<String>,
    pub secret_ciphertext: String,
}

#[derive(Debug, Clone)]
pub struct WebhookSubscriptionDelivery {
    pub id: i64,
    pub subscription_id: WebhookSubscriptionId,
    pub log_id: i64,
    pub payload: String,
    pub attempts: i32,
    /// The server span the source event was written under.
    pub trace: Option<crate::TraceContext>,
}

#[derive(Debug, Clone)]
pub struct WebhookSubscriptionWithSecret {
    pub subscription: WebhookSubscription,
    pub secret_ciphertext: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum SlashHandlerKind {
    Http,
    McpTool,
    /// No-network WASI guest. `handler_target` is an artifact sha256.
    /// Slash-only — FSM hooks reject this kind (the guest is a tool, not
    /// background automation).
    Wasi,
}

impl SlashHandlerKind {
    pub const ALL: &'static [Self] = &[Self::Http, Self::McpTool, Self::Wasi];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::McpTool => "mcp_tool",
            Self::Wasi => "wasi",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "http" => Some(Self::Http),
            "mcp_tool" => Some(Self::McpTool),
            "wasi" => Some(Self::Wasi),
            _ => None,
        }
    }

    /// HTTP and MCP tool handlers may fire on thread FSM edges. WASI
    /// guests are slash tools only.
    pub fn allowed_for_fsm_hooks(self) -> bool {
        matches!(self, Self::Http | Self::McpTool)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SlashCommand {
    pub id: SlashCommandId,
    pub workspace_id: WorkspaceId,
    pub name: String,
    pub description: Option<String>,
    pub handler_kind: SlashHandlerKind,
    pub handler_target: String,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct NewSlashCommand {
    pub workspace_id: WorkspaceId,
    pub name: String,
    pub description: Option<String>,
    pub handler_kind: SlashHandlerKind,
    pub handler_target: String,
    pub secret_ciphertext: String,
}

#[derive(Debug, Clone)]
pub struct SlashCommandWithSecret {
    pub command: SlashCommand,
    pub secret_ciphertext: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct FsmHook {
    pub id: FsmHookId,
    pub workspace_id: WorkspaceId,
    pub label: Option<String>,
    pub from_state: Option<ThreadState>,
    pub to_state: Option<ThreadState>,
    pub handler_kind: SlashHandlerKind,
    pub handler_target: String,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct NewFsmHook {
    pub workspace_id: WorkspaceId,
    pub label: Option<String>,
    pub from_state: Option<ThreadState>,
    pub to_state: Option<ThreadState>,
    pub handler_kind: SlashHandlerKind,
    pub handler_target: String,
    pub secret_ciphertext: String,
}

#[derive(Debug, Clone)]
pub struct FsmHookWithSecret {
    pub hook: FsmHook,
    pub secret_ciphertext: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum AutomationSourceKind {
    SlashCommand,
    FsmHook,
}

impl AutomationSourceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SlashCommand => "slash_command",
            Self::FsmHook => "fsm_hook",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "slash_command" => Some(Self::SlashCommand),
            "fsm_hook" => Some(Self::FsmHook),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct AutomationDelivery {
    pub id: i64,
    pub workspace_id: WorkspaceId,
    pub source_kind: AutomationSourceKind,
    pub source_id: uuid::Uuid,
    pub target_url: String,
    pub header_name: String,
    pub header_value: String,
    pub attempts: i32,
    pub last_error: Option<String>,
    pub delivered_at: Option<DateTime<Utc>>,
    pub quarantined_at: Option<DateTime<Utc>>,
    pub next_attempt_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct WebhookDelivery {
    pub id: i64,
    pub workspace_id: WorkspaceId,
    pub subscription_id: WebhookSubscriptionId,
    pub log_id: i64,
    pub target_url: String,
    pub attempts: i32,
    pub last_error: Option<String>,
    pub delivered_at: Option<DateTime<Utc>>,
    pub quarantined_at: Option<DateTime<Utc>>,
    pub next_attempt_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum OperatorDelivery {
    Automation(AutomationDelivery),
    Webhook(WebhookDelivery),
}

#[derive(Debug, Clone)]
pub struct AutomationDeliveryPending {
    pub id: i64,
    pub workspace_id: WorkspaceId,
    pub source_kind: AutomationSourceKind,
    pub source_id: uuid::Uuid,
    pub target_url: String,
    pub header_name: String,
    pub header_value: String,
    pub payload: String,
    pub attempts: i32,
    /// The server span the triggering request was in, when one was carried.
    pub trace: Option<crate::TraceContext>,
}

#[derive(Debug, Clone)]
pub struct NewAutomationDelivery {
    pub workspace_id: WorkspaceId,
    pub source_kind: AutomationSourceKind,
    pub source_id: uuid::Uuid,
    pub target_url: String,
    pub header_name: String,
    pub header_value: String,
    pub payload: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum ReindexJobStatus {
    Running,
    Completed,
    Failed,
}

/// An embedding reindex job, persisted so its status is visible on any replica
/// and survives restart. `job_id`/`workspace_id` are raw UUIDs to match the
/// operator HTTP shape.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ReindexJob {
    pub job_id: uuid::Uuid,
    pub status: ReindexJobStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workspace_id: Option<uuid::Uuid>,
    pub embedding_model: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub processed: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failed: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub started_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<DateTime<Utc>>,
}

#[cfg(test)]
mod relation_kind_tests {
    use super::*;

    #[test]
    fn controlled_variants_round_trip_as_canonical_snake_case() {
        for (variant, wire) in [
            (RelationKind::Supports, "supports"),
            (RelationKind::Refutes, "refutes"),
            (RelationKind::Defines, "defines"),
            (RelationKind::Depends, "depends"),
            (RelationKind::Duplicates, "duplicates"),
            (RelationKind::Grounds, "grounds"),
            (RelationKind::Supersedes, "supersedes"),
            (RelationKind::SeededFrom, "seeded_from"),
        ] {
            assert_eq!(variant.as_str(), wire);
            assert!(variant.is_controlled());
            assert_eq!(RelationKind::from_wire(wire), variant);
            // serde round-trips through the bare string.
            let json = serde_json::to_string(&variant).unwrap();
            assert_eq!(json, format!("\"{wire}\""));
            assert_eq!(
                serde_json::from_str::<RelationKind>(&json).unwrap(),
                variant
            );
        }
        assert_eq!(RelationKind::CONTROLLED.len(), 8);
    }

    #[test]
    fn unknown_relation_round_trips_verbatim_as_other() {
        let r = RelationKind::from_wire("relates_to");
        assert_eq!(r, RelationKind::Other("relates_to".into()));
        assert!(!r.is_controlled());
        assert_eq!(r.as_str(), "relates_to");
        assert_eq!(serde_json::to_string(&r).unwrap(), "\"relates_to\"");
        assert_eq!(
            serde_json::from_str::<RelationKind>("\"relates_to\"").unwrap(),
            r
        );
        // From<&str> / From<String> are the ergonomic constructors.
        assert_eq!(RelationKind::from("supports"), RelationKind::Supports);
        assert_eq!(
            RelationKind::from("x".to_string()),
            RelationKind::Other("x".into())
        );
    }
}

#[cfg(test)]
mod run_lineage_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn run_id_from_payload_accepts_the_producer_string() {
        assert_eq!(
            run_id_from_payload(&json!({"run_id": "aa4dc966-0e09-44c3-b7a5-2d048b48b301"})),
            Some("aa4dc966-0e09-44c3-b7a5-2d048b48b301")
        );
        assert_eq!(
            run_id_from_payload(&json!({"run_id": "  producer-run  "})),
            Some("producer-run")
        );
        assert_eq!(run_id_from_payload(&json!({"run_id": ""})), None);
        assert_eq!(run_id_from_payload(&json!({"run_id": "   "})), None);
        assert_eq!(run_id_from_payload(&json!({"run_id": 1})), None);
        assert_eq!(run_id_from_payload(&json!({})), None);
        let too_long = "x".repeat(PARENT_RUN_ID_MAX_BYTES + 1);
        assert_eq!(run_id_from_payload(&json!({ "run_id": too_long })), None);
        assert_eq!(normalize_parent_run_id("  ok  "), Some("ok"));
        assert!(normalize_parent_run_id("").is_none());
    }
}

#[cfg(test)]
mod message_serde_tests {
    use super::*;

    fn msg(metadata: serde_json::Value) -> Message {
        Message {
            id: MessageId(uuid::Uuid::nil()),
            thread_id: ThreadId(uuid::Uuid::nil()),
            author_id: MemberId(uuid::Uuid::nil()),
            body: "hi".into(),
            metadata,
            content: None,
            posted_at: chrono::Utc::now(),
            edited_at: None,
            tombstoned_at: None,
        }
    }

    #[test]
    fn empty_metadata_is_omitted_from_the_wire() {
        // {} and null both carry no info → omitted.
        for empty in [serde_json::json!({}), serde_json::Value::Null] {
            let v = serde_json::to_value(msg(empty)).unwrap();
            assert!(
                v.get("metadata").is_none(),
                "empty metadata must be omitted, got {v}"
            );
            // Round-trips back to an (empty) object via default.
            let back: Message = serde_json::from_value(v).unwrap();
            assert!(json_value_is_empty(&back.metadata));
        }
    }

    #[test]
    fn non_empty_metadata_is_kept() {
        let v = serde_json::to_value(msg(serde_json::json!({"topic": "x"}))).unwrap();
        assert_eq!(v["metadata"]["topic"], "x");
    }
}

#[cfg(test)]
mod tool_transcript_tests {
    use super::*;

    fn msg_with(id_seed: u128, ts: i64, blocks: Vec<ContentBlock>) -> Message {
        Message {
            id: MessageId(uuid::Uuid::from_u128(id_seed)),
            thread_id: ThreadId(uuid::Uuid::from_u128(999)),
            author_id: MemberId(uuid::Uuid::from_u128(1)),
            body: String::new(),
            metadata: serde_json::json!({}),
            content: Some(blocks),
            posted_at: DateTime::from_timestamp(ts, 0).unwrap(),
            edited_at: None,
            tombstoned_at: None,
        }
    }
    fn use_block(id: &str, name: &str) -> ContentBlock {
        ContentBlock::ToolUse {
            id: id.into(),
            name: name.into(),
            input: serde_json::json!({"q": 1}),
        }
    }
    fn result_block(id: &str, content: &str, is_error: bool) -> ContentBlock {
        ContentBlock::ToolResult {
            tool_use_id: id.into(),
            content: content.into(),
            is_error,
        }
    }

    #[test]
    fn pairs_use_with_result_across_messages() {
        let thread = ThreadId(uuid::Uuid::from_u128(999));
        let messages = vec![
            msg_with(
                1,
                100,
                vec![use_block("a", "search"), use_block("b", "fetch")],
            ),
            msg_with(
                2,
                200,
                vec![
                    ContentBlock::Text {
                        text: "thinking".into(),
                    },
                    result_block("a", "found 3 rows", false),
                ],
            ),
            msg_with(3, 300, vec![result_block("b", "boom", true)]),
        ];
        let t = tool_transcript(thread, &messages);
        assert_eq!(t.entries.len(), 2, "two tool calls");
        assert!(t.orphan_results.is_empty());

        let a = &t.entries[0];
        assert_eq!(a.tool_use_id, "a");
        assert_eq!(a.name, "search");
        let a_res = a.result.as_ref().expect("a is resolved");
        assert_eq!(a_res.content, "found 3 rows");
        assert!(!a_res.is_error);
        assert_eq!(a_res.message_id, MessageId(uuid::Uuid::from_u128(2)));

        let b = &t.entries[1];
        assert_eq!(b.tool_use_id, "b");
        assert!(b.result.as_ref().unwrap().is_error, "b failed");
    }

    #[test]
    fn unresolved_call_has_no_result_and_orphan_result_is_surfaced() {
        let thread = ThreadId(uuid::Uuid::from_u128(999));
        let messages = vec![
            msg_with(1, 100, vec![use_block("pending", "slow")]),
            msg_with(
                2,
                200,
                vec![result_block("ghost", "no matching call", false)],
            ),
        ];
        let t = tool_transcript(thread, &messages);
        assert_eq!(t.entries.len(), 1);
        assert!(t.entries[0].result.is_none(), "call still pending");
        assert_eq!(t.orphan_results.len(), 1);
        assert_eq!(t.orphan_results[0].tool_use_id, "ghost");
    }

    #[test]
    fn tombstoned_messages_are_skipped() {
        let thread = ThreadId(uuid::Uuid::from_u128(999));
        let mut gone = msg_with(1, 100, vec![use_block("a", "search")]);
        gone.tombstoned_at = Some(Utc::now());
        let messages = vec![gone, msg_with(2, 200, vec![result_block("a", "x", false)])];
        let t = tool_transcript(thread, &messages);
        // The call was tombstoned, so its result has no live match → orphan.
        assert!(t.entries.is_empty());
        assert_eq!(t.orphan_results.len(), 1);
    }
}

#[cfg(test)]
mod budget_tests {
    use super::*;

    fn budget(
        max_tokens: Option<i64>,
        max_usd_micros: Option<i64>,
        max_turns: Option<i64>,
        max_wall_secs: Option<i64>,
        used_tokens: i64,
        used_usd_micros: i64,
        used_turns: i64,
    ) -> ThreadBudget {
        ThreadBudget {
            thread_id: ThreadId(uuid::Uuid::from_u128(1)),
            max_tokens,
            max_usd_micros,
            max_turns,
            max_wall_secs,
            used_tokens,
            used_usd_micros,
            used_turns,
            used_wall_secs: 0,
            used_input_tokens: 0,
            used_output_tokens: 0,
            used_cache_read_tokens: 0,
            used_cache_write_5m_tokens: 0,
            used_cache_write_1h_tokens: 0,
            created_at: DateTime::from_timestamp(0, 0).unwrap(),
            updated_at: DateTime::from_timestamp(0, 0).unwrap(),
        }
    }

    #[test]
    fn no_maxima_never_binds() {
        let b = budget(None, None, None, None, 1_000_000, 1_000_000, 1_000_000);
        assert_eq!(b.exceeded(None), None);
        assert_eq!(b.exceeded(Some(1_000_000)), None);
    }

    #[test]
    fn each_dimension_binds_at_or_over_its_max() {
        assert_eq!(
            budget(Some(10), None, None, None, 10, 0, 0).exceeded(None),
            Some(BudgetReason::Tokens)
        );
        assert_eq!(
            budget(None, Some(10), None, None, 0, 11, 0).exceeded(None),
            Some(BudgetReason::Usd)
        );
        assert_eq!(
            budget(None, None, Some(3), None, 0, 0, 3).exceeded(None),
            Some(BudgetReason::Turns)
        );
        assert_eq!(
            budget(None, None, None, Some(60), 0, 0, 0).exceeded(Some(60)),
            Some(BudgetReason::Wall)
        );
        // Just under each does not bind.
        assert_eq!(
            budget(Some(10), None, None, None, 9, 0, 0).exceeded(None),
            None
        );
        assert_eq!(
            budget(None, None, None, Some(60), 0, 0, 0).exceeded(Some(59)),
            None
        );
    }

    #[test]
    fn checked_in_fixed_order_tokens_first() {
        // Both tokens and turns over → tokens wins (checked first).
        let b = budget(Some(1), None, Some(1), None, 5, 0, 5);
        assert_eq!(b.exceeded(None), Some(BudgetReason::Tokens));
    }

    #[test]
    fn non_positive_max_never_binds() {
        // A zero/negative maximum is treated as unbounded (guards a nonsense set).
        assert_eq!(
            budget(Some(0), None, None, None, 100, 0, 0).exceeded(None),
            None
        );
    }

    #[test]
    fn wall_only_checked_when_working() {
        let b = budget(None, None, None, Some(60), 0, 0, 0);
        assert_eq!(b.exceeded(None), None, "not working → no wall check");
        assert_eq!(b.exceeded(Some(60)), Some(BudgetReason::Wall));
    }

    #[test]
    fn charged_wall_time_counts_with_the_live_claim() {
        let b = ThreadBudget {
            used_wall_secs: 40,
            ..budget(None, None, None, Some(60), 0, 0, 0)
        };
        assert_eq!(b.exceeded(None), None, "40 of 60 charged");
        assert_eq!(b.exceeded(Some(19)), None);
        assert_eq!(b.exceeded(Some(20)), Some(BudgetReason::Wall));
        let spent = ThreadBudget {
            used_wall_secs: 60,
            ..b.clone()
        };
        assert_eq!(
            spent.exceeded(None),
            Some(BudgetReason::Wall),
            "charged time alone binds, with no claim working"
        );
        assert_eq!(
            b.exceeded(Some(-100)),
            None,
            "a clock behind the start charges nothing back"
        );
    }

    #[test]
    fn reason_as_str_roundtrip() {
        assert_eq!(BudgetReason::Tokens.as_str(), "tokens");
        assert_eq!(BudgetReason::Usd.as_str(), "usd");
        assert_eq!(BudgetReason::Turns.as_str(), "turns");
        assert_eq!(BudgetReason::Wall.as_str(), "wall");
    }
}

#[cfg(test)]
mod notification_group_tests {
    use super::*;

    fn note(thread: Option<u128>, ts: i64, read: bool) -> Notification {
        Notification {
            id: NotificationId(uuid::Uuid::new_v4()),
            workspace_id: WorkspaceId(uuid::Uuid::from_u128(1)),
            member_id: MemberId(uuid::Uuid::from_u128(2)),
            kind: crate::EventKind::MentionRecorded,
            source_log_id: ts,
            channel_id: None,
            thread_id: thread.map(|t| ThreadId(uuid::Uuid::from_u128(t))),
            message_id: None,
            actor_id: None,
            created_at: DateTime::from_timestamp(ts, 0).unwrap(),
            read_at: read.then(|| DateTime::from_timestamp(ts, 0).unwrap()),
            snoozed_until: None,
        }
    }

    #[test]
    fn groups_by_thread_with_counts_and_latest() {
        // Newest-first input (as a Notification list arrives).
        let notes = vec![
            note(Some(10), 300, false), // thread A, newest, unread
            note(Some(20), 250, true),  // thread B, read
            note(Some(10), 200, true),  // thread A, older, read
            note(None, 150, false),     // no thread, unread
        ];
        let groups = group_notifications_by_thread(&notes);
        assert_eq!(groups.len(), 3, "A, B, and the no-thread group");

        // Ordered by latest activity: A (300) > B (250) > none (150).
        assert_eq!(
            groups[0].thread_id,
            Some(ThreadId(uuid::Uuid::from_u128(10)))
        );
        assert_eq!(groups[0].count, 2);
        assert_eq!(groups[0].unread_count, 1);
        assert_eq!(groups[0].latest.created_at.timestamp(), 300);

        assert_eq!(
            groups[1].thread_id,
            Some(ThreadId(uuid::Uuid::from_u128(20)))
        );
        assert_eq!(groups[1].count, 1);
        assert_eq!(groups[1].unread_count, 0);

        assert_eq!(groups[2].thread_id, None);
        assert_eq!(groups[2].count, 1);
        assert_eq!(groups[2].unread_count, 1);
    }

    #[test]
    fn empty_input_yields_no_groups() {
        assert!(group_notifications_by_thread(&[]).is_empty());
    }
}

#[cfg(test)]
mod result_kind_from_payload_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extracts_the_namespaced_string() {
        assert_eq!(
            result_kind_from_payload(&json!({"result_kind": "example.review.result/1"})),
            Some("example.review.result/1")
        );
    }

    #[test]
    fn ignores_missing_empty_and_non_string() {
        assert_eq!(result_kind_from_payload(&json!({})), None);
        assert_eq!(result_kind_from_payload(&json!({"result_kind": ""})), None);
        assert_eq!(
            result_kind_from_payload(&json!({"result_kind": "  "})),
            None
        );
        assert_eq!(result_kind_from_payload(&json!({"result_kind": 1})), None);
        assert_eq!(
            result_kind_from_payload(&json!({"kind": "decision"})),
            None,
            "the old ADR `kind` field is not the search facet"
        );
    }

    #[test]
    fn does_not_require_the_waiter_schema() {
        assert_eq!(
            result_kind_from_payload(&json!({"result_kind": "acme.plan.result/2"})),
            Some("acme.plan.result/2")
        );
    }
}

#[cfg(test)]
mod blocked_reason_tests {
    use super::*;

    #[test]
    fn all_variants_round_trip() {
        for &reason in BlockedReason::ALL {
            match reason {
                BlockedReason::Dag
                | BlockedReason::Gate
                | BlockedReason::Human
                | BlockedReason::Child
                | BlockedReason::Quota
                | BlockedReason::Unclaimable => {}
            }
            assert_eq!(
                BlockedReason::parse(reason.as_str()),
                Some(reason),
                "as_str/parse round-trip broken for {reason:?}"
            );
            let json = serde_json::to_string(&reason).unwrap();
            assert_eq!(json, format!("\"{}\"", reason.as_str()));
            assert_eq!(
                serde_json::from_str::<BlockedReason>(&json).unwrap(),
                reason
            );
        }
        assert_eq!(BlockedReason::ALL.len(), 6);
    }

    #[test]
    fn unknown_reason_does_not_parse() {
        assert_eq!(BlockedReason::parse("stuck"), None);
        assert_eq!(BlockedReason::parse("decision"), None);
        assert!(serde_json::from_str::<BlockedReason>("\"stuck\"").is_err());
    }
}

#[cfg(test)]
mod budget_limits_strictness_tests {
    use super::BudgetLimits;

    /// A misspelled dimension must not read as an omission.
    ///
    /// The write is a replace — omitting a dimension *removes* that limit — so
    /// before this, `max_wall_seconds` deserialized to `max_wall_secs: None`
    /// and silently disarmed the wall cap, returning `200` with the budget
    /// echoed back. A caller had no way to tell that from success.
    #[test]
    fn a_misspelled_dimension_is_rejected_rather_than_silently_dropped() {
        let typo = serde_json::json!({ "max_tokens": 100, "max_wall_seconds": 3600 });
        let err = serde_json::from_value::<BudgetLimits>(typo)
            .expect_err("a misspelled dimension must not parse");
        assert!(
            err.to_string().contains("max_wall_seconds"),
            "the error should name the offending key, got: {err}"
        );
    }

    /// Omission itself still means "unbounded" — the documented semantics are
    /// unchanged, only typos are now distinguishable from them.
    #[test]
    fn omitting_a_dimension_still_means_unbounded() {
        let partial = serde_json::json!({ "max_tokens": 100 });
        let limits: BudgetLimits = serde_json::from_value(partial).expect("parses");
        assert_eq!(limits.max_tokens, Some(100));
        assert_eq!(
            limits.max_wall_secs, None,
            "omission still leaves it unbound"
        );

        // And an explicit null is accepted, so a caller can clear on purpose.
        let explicit = serde_json::json!({ "max_tokens": 100, "max_wall_secs": null });
        let limits: BudgetLimits = serde_json::from_value(explicit).expect("parses");
        assert_eq!(limits.max_wall_secs, None);
    }
}

#[cfg(test)]
mod vote_kind_tests {
    use super::VoteKind;

    #[test]
    fn closed_set_parses_and_rejects_strings_outside_it() {
        assert_eq!(
            VoteKind::ALL.map(VoteKind::as_str),
            ["approve", "request_changes", "ack"]
        );
        for kind in VoteKind::ALL {
            assert_eq!(VoteKind::parse(kind.as_str()), Some(kind));
            let wire = serde_json::to_value(kind).unwrap();
            assert_eq!(wire, serde_json::json!(kind.as_str()));
            assert_eq!(serde_json::from_value::<VoteKind>(wire).unwrap(), kind);
        }
        for rejected in ["up", "upvote", "request-changes", "ship", ""] {
            assert_eq!(VoteKind::parse(rejected), None, "{rejected}");
            assert!(
                serde_json::from_value::<VoteKind>(serde_json::json!(rejected)).is_err(),
                "{rejected} parsed"
            );
        }
    }
}

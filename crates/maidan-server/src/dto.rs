//! Request DTOs. Response shapes use the `maidan_types` models directly
//! since they already derive `Serialize`. The DTOs here mirror the
//! domain `New<X>` structs but exist so the HTTP boundary can evolve
//! independently of the storage layer (e.g., omitting `workspace_id` in
//! a nested route in favor of the path parameter).

use chrono::{DateTime, Utc};
use maidan_types::{
    ApiTokenId, AppId, AppInstallationId, ApprovalGate, ArtifactKind, BlockedReason, ChannelId,
    ChannelMemberRole, ContentBlock, DelegationGrantId, EgressSurface, EmailDeliveryMode,
    EscalationPolicy, EventKind, FsmHookId, LandColor, LandGateStatus, MemberFreeze, MemberId,
    MemberKind, PeerId, RecipeSpec, RefSide, RelationKind, ReviewDecision, ShareTicket,
    SlashCommandId, SlashHandlerKind, ThreadDependency, ThreadId, TokenPolicy, TokenQuota,
    WebhookSubscriptionId, WorkspaceId,
};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateWorkspace {
    pub name: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct EraseWorkspace {
    pub confirm_workspace_id: uuid::Uuid,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateMember {
    pub handle: String,
    pub display_name: Option<String>,
    pub kind: MemberKind,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateChannel {
    pub name: String,
    pub topic: Option<String>,
    #[serde(default)]
    pub private: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AddChannelMember {
    pub member_id: uuid::Uuid,
    /// `member` (default) or `admin`.
    pub role: Option<ChannelMemberRole>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateThread {
    pub title: Option<String>,
    pub parent_thread_id: Option<uuid::Uuid>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct TransitionThread {
    pub action: String,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AssignThread {
    pub assignee_id: uuid::Uuid,
    /// Optional handoff note for the assignee.
    #[serde(default)]
    pub note: Option<String>,
}

/// Set a thread's durable owner — the accountable party, distinct from the
/// assignee/claimer.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SetThreadOwner {
    pub owner_id: uuid::Uuid,
}

/// Rename a thread. The title is required and non-empty — a rename gives the
/// thread a name, so a blank title is rejected.
#[derive(Debug, Deserialize, ToSchema)]
pub struct RenameThread {
    pub title: String,
}

/// Atomically claim an unassigned thread for a member. The claimer is both the
/// actor and the assignee.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ClaimThread {}

/// Claim the next unassigned/expired thread in a channel.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ClaimNextThread {
    /// Optional lease deadline in seconds; the claim is reclaimable after it
    /// lapses. Omit for a durable claim.
    #[serde(default)]
    pub lease_secs: Option<i64>,
}

/// Extend a claimed thread's lease, for the current assignee holding the
/// matching fencing token. `claim_lease_id` is the value returned in the
/// claiming response's `Thread.claim_lease_id`; a stale holder whose claim was
/// reclaimed presents an outdated token and is rejected.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RenewClaim {
    pub claim_lease_id: uuid::Uuid,
    pub lease_secs: i64,
}

/// Acknowledge a claim and start the working clock. The current holder presents
/// its fencing token; the working clock (`work_started_at`) is stamped once and
/// preserved on re-acknowledge.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct AcknowledgeClaim {
    pub claim_lease_id: uuid::Uuid,
}

/// Release a claim (graceful handoff): the current holder returns the thread to
/// the queue by presenting its fencing token.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ReleaseClaim {
    pub claim_lease_id: uuid::Uuid,
}

/// Add a task-dependency edge — the thread in the path depends on
/// `depends_on_thread_id`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct AddThreadDependency {
    pub depends_on_thread_id: uuid::Uuid,
}

/// Create a task schedule. When due, the sweeper creates a thread titled
/// `title` in `channel_id`. `interval_secs` omitted (or null) = one-shot; a
/// positive value = recurring. `first_run_at` omitted = fire on the next tick
/// (defaults to now).
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateTaskSchedule {
    pub channel_id: uuid::Uuid,
    pub title: String,
    #[serde(default)]
    pub interval_secs: Option<i64>,
    #[serde(default)]
    pub first_run_at: Option<chrono::DateTime<chrono::Utc>>,
    /// When set, each firing instantiates this recipe (parent + DAG children)
    /// instead of creating one bare thread.
    #[serde(default)]
    pub recipe_id: Option<uuid::Uuid>,
}

/// Create a recipe blueprint. `spec` is the `RecipeSpec` (params, definition of
/// done, retry, inline DAG children).
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateRecipe {
    pub channel_id: uuid::Uuid,
    pub name: String,
    pub spec: RecipeSpec,
}

/// Instantiate a recipe: `params` are validated against the recipe's declared
/// params (required ones must be present).
#[derive(Debug, Deserialize, ToSchema)]
pub struct InstantiateRecipe {
    #[serde(default)]
    #[schema(value_type = Object)]
    pub params: serde_json::Value,
}

/// Create (or rotate) a named secret. The `value` is encrypted at rest and
/// never returned by `list` — only by `resolve`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateSecret {
    pub name: String,
    pub value: String,
}

/// Create a memory block. `label` is the block's within-workspace key; `value`
/// defaults to empty, `read_only` to false. Creating an existing label returns
/// the existing block (concurrent-safe).
#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateMemoryBlock {
    pub label: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub char_limit: Option<i64>,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default)]
    pub value: Option<String>,
}

/// Full-rewrite a memory block's value. A read-only block or a value over the
/// block's char limit is rejected (400).
#[derive(Debug, Deserialize, ToSchema)]
pub struct SetMemoryBlockValue {
    pub value: String,
}

/// Set a thread's review requirement: `required_count` distinct qualifying
/// approvals before it can `close`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SetReviewRequirement {
    pub required_count: i64,
}

/// Name a reviewer for a thread — the eligible set (empty = open review).
#[derive(Debug, Deserialize, ToSchema)]
pub struct AddReviewer {
    pub member_id: uuid::Uuid,
}

/// Submit a review decision. The reviewer is the caller; an owner/assignee may
/// submit but it won't count toward the requirement.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SubmitReview {
    pub decision: ReviewDecision,
    #[serde(default)]
    pub note: Option<String>,
}

/// Record a LandGate pointer. `land` is optional — a fail is always red; a pass
/// defaults to green; amber is flags-then-still-engages and is not a land.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SetLandGate {
    pub status: LandGateStatus,
    #[serde(default)]
    pub artifact_sha: Option<String>,
    #[serde(default)]
    pub land: Option<LandColor>,
}

/// A resolved secret value — the `resolve` response body. This is the only
/// place a secret value crosses the wire out of Maidan.
#[derive(Debug, Serialize, ToSchema)]
pub struct SecretValue {
    pub name: String,
    pub value: String,
}

/// Freeze a member — the kill-switch. `reason` is an optional audit note.
#[derive(Debug, Deserialize, ToSchema)]
pub struct FreezeMember {
    #[serde(default)]
    pub reason: Option<String>,
}

/// The result of freezing a member: the freeze record + the number of active
/// claims released (leases dropped).
#[derive(Debug, Serialize, ToSchema)]
pub struct FreezeResult {
    pub freeze: MemberFreeze,
    pub released: u64,
}

/// Pause (`false`) or resume (`true`) a schedule.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SetTaskScheduleActive {
    pub active: bool,
}

/// Add a skill — to a member (`declares`) or a thread (`requires`).
#[derive(Debug, Deserialize, ToSchema)]
pub struct AddSkill {
    pub skill: String,
}

/// Define (or redefine) a glossary term. The term itself is the path segment;
/// this is the body. `aliases` defaults to empty when omitted.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SetGlossaryTerm {
    pub definition: String,
    #[serde(default)]
    pub aliases: Option<Vec<String>>,
}

/// Set a task's structured result. `result` is arbitrary JSON.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SetThreadResult {
    #[schema(value_type = Object)]
    pub result: serde_json::Value,
}

/// Set a thread's persisted steer — durable steering guidance.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SetThreadSteer {
    pub steer: String,
}

/// Home a producer's `run_id` on a thread as `parent_run_id`. The value is the
/// producer's string — Maidan does not mint a parallel id.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SetThreadLineage {
    pub parent_run_id: String,
}

/// Query params for workspace run-lineage reads.
#[derive(Debug, Deserialize, ToSchema, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct RunLineageQuery {
    /// The producer `run_id` (e.g. The waiter envelope). Empty / missing /
    /// whitespace is 400 after capability check. Not a minted Maidan id.
    #[serde(default)]
    pub parent_run_id: String,
}

/// Answer a human-approval gate: accept / decline / cancel, with the HMAC
/// `request_state` the server issued alongside the pending gate.
#[derive(Debug, Deserialize, ToSchema)]
pub struct AnswerApprovalGate {
    /// The opaque integrity token from the pending-gate list — must verify.
    pub request_state: String,
    /// `accept`, `decline`, or `cancel`. Any other value is a bad request — an
    /// empty or unknown action is never a silent accept.
    pub action: String,
    /// Optional structured detail the human supplies with their decision.
    #[serde(default)]
    #[schema(value_type = Option<Object>)]
    pub content: Option<serde_json::Value>,
}

/// A pending approval gate plus the `request_state` a human echoes back to
/// answer it.
#[derive(Debug, Serialize, ToSchema)]
pub struct ApprovalGateView {
    pub gate: ApprovalGate,
    pub request_state: String,
}

/// A task's dependency edges plus whether it is ready to run.
#[derive(Debug, Serialize, ToSchema)]
pub struct ThreadDependenciesView {
    pub dependencies: Vec<ThreadDependency>,
    /// True when every dependency is terminal (closed/archived) — the task is
    /// ready to claim.
    pub ready: bool,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct UnassignThread {}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateMessage {
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub metadata: serde_json::Value,
    /// Typed structured content. When present and `body` is empty, the server
    /// derives `body` from these blocks for search/back-compat.
    #[serde(default)]
    pub content: Option<Vec<ContentBlock>>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct EditMessageRequest {
    #[serde(default)]
    pub body: String,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
    /// Replacement structured content. Omitted → keep existing.
    #[serde(default)]
    pub content: Option<Vec<ContentBlock>>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateMention {
    pub member_id: uuid::Uuid,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateVote {
    pub kind: String,
    /// Optional confidence weight, by convention in `0..=1`, for weighted
    /// consensus. Omit to state no confidence.
    #[serde(default)]
    pub confidence: Option<f64>,
}

/// Seed a new work thread from a source message — the write side of "re-ask".
/// `inclusion`: `pointer` (default, lineage edge only) or `quote` (the seed's
/// first message quotes the source). `channel_id` defaults to the source's
/// channel. The seed is a titled, claimable child; the source is untouched;
/// lineage is a `seeded_from` reference edge (new thread → source message).
#[derive(Debug, Deserialize, ToSchema)]
pub struct SeedFromMessage {
    pub title: String,
    #[serde(default)]
    pub inclusion: Option<String>,
    #[serde(default)]
    pub channel_id: Option<uuid::Uuid>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateReaction {
    pub emoji: String,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RemoveReaction {
    pub emoji: String,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PinMessage {
    pub message_id: uuid::Uuid,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateReference {
    pub src_kind: RefSide,
    pub src_id: uuid::Uuid,
    pub dst_kind: RefSide,
    pub dst_id: uuid::Uuid,
    /// Typed relation. Snake_case string; controlled set
    /// `RelationKind::CONTROLLED`, unknown values round-trip via `Other`.
    #[schema(value_type = String)]
    pub relation: RelationKind,
}

#[derive(Debug, Deserialize, ToSchema, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ThreadContextQuery {
    #[serde(default = "default_limit")]
    pub message_limit: i64,
    #[serde(default = "default_transition_limit")]
    pub transition_limit: i64,
    pub message_cursor: Option<uuid::Uuid>,
    /// Include full `body_before`/`body_after` on each edit (heavy). Default
    /// `false` returns edit metadata only — the largest token lever on a pack.
    #[serde(default)]
    pub include_edits: bool,
    /// Include the workspace glossary (canonical term definitions) so the pack is
    /// grounded in shared vocabulary. Default `true`; omitted from the response
    /// when the glossary is empty. Set `false` to drop it for a token-tight pack.
    #[serde(default = "default_true")]
    pub include_glossary: bool,
    /// As-of context replay: reconstruct the thread as it stood at this
    /// event-log id, deterministic over the immutable log. Omit for the live
    /// pack. An unknown id is `404`.
    pub as_of: Option<i64>,
    /// Token budget for the message page. When set, a page over budget is
    /// folded — the opening message and the recent tail are kept, the middle is
    /// elided into an auditable `elision` marker on the response. Omit to cap
    /// by rows only.
    pub token_budget: Option<i64>,
    /// Attach parent grounding to a child thread's pack: the parent's opening
    /// ask + latest decision, orienting a fresh claimer. Default `true`; absent
    /// for root threads and withheld for a cross-channel or DM parent. Set
    /// `false` for the leanest possible pack.
    #[serde(default = "default_true")]
    pub include_parent_grounding: bool,
    /// Attach in-channel accepted/closed decisions so a fresh claimer sees what
    /// the channel already decided. Default `true`; omitted when empty. Waiter
    /// envelopes (`maidan.waiter.result/1`) appear only when `status` is
    /// `reviewed`. `result_kind` is a namespaced string, not a closed enum. Set
    /// `false` for the leanest pack. Withheld on DM channels.
    #[serde(default = "default_true")]
    pub include_accepted_decisions: bool,
}

/// Query for `GET /threads/:id/tool-transcript`.
#[derive(Debug, Deserialize, ToSchema, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ToolTranscriptQuery {
    /// Max messages to scan (default 200, clamped 1..=500).
    pub limit: Option<i64>,
}

/// Body for `PUT /workspaces/:wid/wip-limit`. `limit: null` clears the cap
/// (unlimited); a value caps concurrent live claims per member (`0` freezes
/// claiming).
#[derive(Debug, Deserialize, ToSchema)]
pub struct SetWipLimit {
    pub limit: Option<i64>,
}

/// Body for `PUT /workspaces/:wid/delegation-policy`. `max_grant_days` is the
/// longest a delegation grant may live (1–3650); `null` restores the default,
/// 90 days.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SetDelegationPolicy {
    pub max_grant_days: Option<i64>,
}

/// Body for `PUT /threads/:id/unclaimable` — park a thread from dispatch with a
/// reason (must be non-empty).
#[derive(Debug, Deserialize, ToSchema)]
pub struct MarkUnclaimable {
    pub reason: String,
}

/// Body for `PUT /threads/:id/block` — set an explicit dispatch block. `reason`
/// is the closed `BlockedReason` enum
/// (`dag|gate|human|child|quota|unclaimable`); unknown → 400 at the extractor.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SetThreadBlock {
    pub reason: BlockedReason,
}

/// Body for `PUT /threads/:id/wait` — set a wait timer. On `wait_until` the
/// sweeper escalates via `on_timeout` (default `notify`); an optional `reason`
/// records why the thread is waiting.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SetThreadWait {
    pub wait_until: chrono::DateTime<chrono::Utc>,
    #[serde(default)]
    pub on_timeout: Option<EscalationPolicy>,
    #[serde(default)]
    pub reason: Option<String>,
}

/// Body for `PUT /threads/:id/priority` — set a thread's dispatch priority.
/// Higher = more urgent; the default is 0.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SetThreadPriority {
    pub priority: i64,
}

/// Body for `PUT /workspaces/:id/legal-hold` — place a legal hold. `reason` is
/// required (non-empty).
#[derive(Debug, Deserialize, ToSchema)]
pub struct PlaceLegalHold {
    pub reason: String,
}

/// The workspace's WIP limit; `null` when unset (unlimited).
#[derive(Debug, Serialize, ToSchema)]
pub struct WipLimitView {
    pub limit: Option<i64>,
}

/// A member's WIP status: current live-claim count + the workspace limit
/// (`null` = unlimited).
#[derive(Debug, Serialize, ToSchema)]
pub struct MemberWipView {
    pub live_claims: i64,
    pub limit: Option<i64>,
}

/// Body for `PUT /workspaces/:id/spawn-budget` — the workspace's cap on agent
/// fan-out. A full replace: an omitted or `null` axis is unlimited, so `{}`
/// clears the budget. `0` freezes an axis.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SetSpawnBudget {
    /// Max direct child threads per parent.
    #[serde(default)]
    pub max_children: Option<i64>,
    /// Max thread nesting depth (a root thread is depth 1).
    #[serde(default)]
    pub max_depth: Option<i64>,
    /// Max tool calls recorded on one thread.
    #[serde(default)]
    pub max_tools: Option<i64>,
}

/// The workspace's spawn budget; a `null` axis is unlimited.
#[derive(Debug, Serialize, ToSchema)]
pub struct SpawnBudgetView {
    pub max_children: Option<i64>,
    pub max_depth: Option<i64>,
    pub max_tools: Option<i64>,
}

/// Query for a channel's agent-work DLQ.
#[derive(Debug, Deserialize, ToSchema, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct DlqQuery {
    /// Max entries to return (default 50, clamped 1..=200).
    pub limit: Option<i64>,
}

fn default_transition_limit() -> i64 {
    50
}

#[derive(Debug, Deserialize, ToSchema, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct WorkspaceContextQuery {
    #[serde(default = "default_workspace_thread_limit")]
    pub thread_limit: i64,
    #[serde(default)]
    pub message_limit: i64,
    #[serde(default)]
    pub transition_limit: i64,
    pub thread_cursor: Option<uuid::Uuid>,
    /// Include full edit bodies on every nested thread pack (heavy). Default
    /// `false` returns edit metadata only.
    #[serde(default)]
    pub include_edits: bool,
    /// Include the workspace glossary once at the top level (grounding). Default
    /// `true`; omitted when empty. Set `false` to drop it.
    #[serde(default = "default_true")]
    pub include_glossary: bool,
    /// Token budget applied to **each** nested thread's message page. Omit for
    /// row-only caps.
    pub token_budget: Option<i64>,
}

fn default_workspace_thread_limit() -> i64 {
    10
}

#[derive(Debug, Deserialize, ToSchema, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListMessagesQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
}

/// Query for `GET /channels/:cid/threads` — keyset pagination over a channel's
/// live threads, `(created_at, id)` ascending. `limit` defaults to 100 (clamped
/// 1..=500); `cursor` is the prior page's last thread id (exclusive).
#[derive(Debug, Deserialize, ToSchema, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListThreadsQuery {
    pub limit: Option<i64>,
    pub cursor: Option<uuid::Uuid>,
}

#[derive(Debug, Deserialize, ToSchema, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListMessageEditsQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
}

fn default_limit() -> i64 {
    100
}

fn default_true() -> bool {
    true
}

#[derive(Debug, Deserialize, ToSchema, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListReferencesQuery {
    /// List references FROM this source (forward edges). Give either the
    /// `src_*` pair OR the `dst_*` pair.
    pub src_kind: Option<RefSide>,
    pub src_id: Option<uuid::Uuid>,
    /// List references TO this target (reverse edges — "what references this").
    pub dst_kind: Option<RefSide>,
    pub dst_id: Option<uuid::Uuid>,
    /// Optional relation filter (a `RelationKind` wire string, e.g. `refutes`).
    #[schema(value_type = Option<String>)]
    pub relation: Option<RelationKind>,
}

#[derive(Debug, Deserialize, ToSchema, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListEventsQuery {
    #[serde(default)]
    pub after_id: i64,
    #[serde(default = "default_limit")]
    pub limit: i64,
    /// Projector shape: restrict to one channel.
    #[serde(default)]
    pub channel_id: Option<uuid::Uuid>,
    /// Projector shape: restrict to one thread.
    #[serde(default)]
    pub thread_id: Option<uuid::Uuid>,
    /// Comma-separated event kinds (`message_posted,thread_ready`). Empty/absent = all.
    /// Unknown tokens fail loud (400), never silently dropped.
    #[serde(default)]
    pub types: Option<String>,
    /// Durable delivery-cursor key. Floors `after_id` to the stored watermark;
    /// a too-old watermark is 409 `must_refetch`, not a clamp.
    #[serde(default)]
    pub consumer_id: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct LogSnapshotQuery {
    /// Include the domain graph. Requires `token:admin` (or a federation
    /// peer). Default false — header and `graph_hash` only.
    #[serde(default)]
    pub include_graph: bool,
}

#[derive(Debug, Deserialize, ToSchema, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct CatchUpQuery {
    /// Exclusive cursor: events have `id > after_lsn`.
    #[serde(default)]
    pub after_lsn: i64,
    #[serde(default = "default_limit")]
    pub limit: i64,
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListAuditQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SearchMode {
    #[default]
    Lexical,
    /// Embed `q` with the configured provider and rank by cosine similarity.
    Semantic,
    /// Run lexical + semantic and fuse their normalized `[0,1]` scores
    /// (`hybrid_weight` controls the semantic share).
    Hybrid,
}

#[derive(Debug, Deserialize, ToSchema, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct SearchQuery {
    pub q: String,
    #[serde(default)]
    pub mode: SearchMode,
    #[serde(default = "default_search_limit")]
    pub limit: i64,
    /// Restrict hits to messages by this member.
    pub author: Option<uuid::Uuid>,
    /// Restrict hits to messages in threads under this channel.
    pub channel: Option<uuid::Uuid>,
    /// Restrict hits to messages whose author has this kind (`human` / `agent`).
    pub kind: Option<MemberKind>,
    /// Date-range facet: only messages posted at/after this RFC 3339 instant
    /// (inclusive lower bound).
    pub after: Option<chrono::DateTime<chrono::Utc>>,
    /// Date-range facet: only messages posted strictly before this RFC 3339
    /// instant (exclusive upper bound — a half-open window).
    pub before: Option<chrono::DateTime<chrono::Utc>>,
    /// Semantic only: query this model's embedding table (default: active provider).
    pub embedding_model: Option<String>,
    /// Hybrid only: semantic weight in `[0,1]` (default `0.5`). `combined =
    /// w*semantic + (1-w)*lexical` over the normalized scores.
    pub hybrid_weight: Option<f64>,
    /// Drop the full message `body` from each hit, returning only the bounded
    /// `snippet` (semantic hits get a truncated body prefix as their snippet).
    /// Default `false` keeps today's full-body response.
    #[serde(default)]
    pub snippet_only: bool,
}

fn default_search_limit() -> i64 {
    25
}

#[derive(Debug, Deserialize, ToSchema, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListMentionsQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
}

#[derive(Debug, Deserialize, ToSchema, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListInboxQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
}

#[derive(Debug, Deserialize, ToSchema, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListNotificationsQuery {
    /// When true, only unread notifications are returned.
    #[serde(default)]
    pub unread_only: bool,
    #[serde(default = "default_limit")]
    pub limit: i64,
}

/// The unread-notification badge count for a member.
#[derive(Debug, Serialize, ToSchema)]
pub struct UnreadCount {
    pub count: i64,
}

/// Snooze a notification until this RFC 3339 instant.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SnoozeNotification {
    pub until: chrono::DateTime<chrono::Utc>,
}

/// Query for a member's buried decisions.
#[derive(Debug, Deserialize, ToSchema, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct DecisionsQuery {
    /// Only decisions produced after this RFC 3339 instant (default: 7 days ago).
    pub since: Option<chrono::DateTime<chrono::Utc>>,
    /// Max decisions to return (default 50, clamp 1..=200).
    pub limit: Option<i64>,
}

/// Query for a member's notification-backed manager digest.
#[derive(Debug, Deserialize, ToSchema, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ManagerDigestQuery {
    /// Only unread lifecycle notifications created after this instant
    /// (default: 7 days ago).
    pub since: Option<chrono::DateTime<chrono::Utc>>,
}

/// Query params for `GET /workspaces/:id/tombstones`.
#[derive(Debug, Deserialize, ToSchema, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListTombstonesQuery {
    pub channel_id: Option<uuid::Uuid>,
    pub thread_id: Option<uuid::Uuid>,
    /// Include hard-purged reconstructions from `MessageTombstoned` events.
    #[serde(default)]
    pub include_purged: bool,
    /// Max rows (default 100, clamp 1..=500).
    pub limit: Option<i64>,
}

/// Query params for `GET /workspaces/:id/kind-census`.
#[derive(Debug, Deserialize, ToSchema, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct KindCensusQuery {
    pub channel_id: Option<uuid::Uuid>,
    pub thread_id: Option<uuid::Uuid>,
}

/// Query params for `GET /workspaces/:id/results`.
#[derive(Debug, Deserialize, ToSchema, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ListThreadResultsQuery {
    /// Exact-match facet on the namespaced `result_kind` string (e.g.
    /// `example.review.result/1`). Absent = every non-tombstoned result in the
    /// workspace. Not a closed enum.
    pub result_kind: Option<String>,
    /// Max results to return (default 50, clamp 1..=500).
    pub limit: Option<i64>,
}

/// Query params for workspace import.
#[derive(Debug, Deserialize, ToSchema, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ImportQuery {
    /// `new` (default) remaps every id to a fresh one and lands the content as a
    /// brand-new workspace; `restore` preserves the bundle's ids verbatim.
    #[serde(default)]
    pub mode: ImportMode,
    /// For `restore` only: overwrite an existing workspace with the same id by
    /// erasing it first. Ignored in `new` mode. A no-op if nothing exists.
    #[serde(default)]
    pub force: bool,
}

/// Import mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, Default)]
#[serde(rename_all = "snake_case")]
pub enum ImportMode {
    /// Remap all ids; land as a new workspace.
    #[default]
    New,
    /// Preserve ids; restore into the same identities.
    Restore,
}

/// Result of a workspace import: the id of the workspace that now holds the
/// content (a fresh id in `new` mode, the bundle's id in `restore`).
#[derive(Debug, Serialize, ToSchema)]
pub struct ImportResult {
    pub workspace_id: uuid::Uuid,
    pub mode: ImportMode,
}

/// Outcome of `POST /workspaces/export/verify`. The inner graph is not
/// imported.
#[derive(Debug, Serialize, ToSchema)]
pub struct VerifyExportResult {
    pub ok: bool,
    pub token_policy: TokenPolicy,
    pub public_key: String,
    pub content_sha256: String,
    pub workspace_id: Option<uuid::Uuid>,
}

/// `GET /operator/export-public-key` — out-of-band pin for a destination
/// instance's `MAIDAN_EXPORT_VERIFY_KEYS`.
#[derive(Debug, Serialize, ToSchema)]
pub struct ExportPublicKey {
    pub alg: String,
    pub public_key: String,
    pub token_policy: TokenPolicy,
}

/// Result of marking all of a member's notifications read.
#[derive(Debug, Serialize, ToSchema)]
pub struct MarkAllRead {
    pub cleared: i64,
}

/// Set a member's mute preference for one event kind.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SetNotificationPref {
    pub kind: EventKind,
    pub muted: bool,
}

/// Follow a channel.
#[derive(Debug, Deserialize, ToSchema)]
pub struct FollowChannel {
    pub channel_id: ChannelId,
}

/// Follow a thread.
#[derive(Debug, Deserialize, ToSchema)]
pub struct FollowThread {
    pub thread_id: ThreadId,
}

/// Follow another member's work occupancy.
#[derive(Debug, Deserialize, ToSchema)]
pub struct FollowMember {
    pub followed_member_id: MemberId,
}

/// Set a member's delivery email address.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SetEmail {
    pub email: String,
}

/// Query for `GET /members/:id/waiting` — the SLA in seconds an item may wait
/// before it is flagged overdue (default 86400 = 24h).
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct WaitingQuery {
    #[serde(default)]
    pub sla_secs: Option<i64>,
}

/// The keys of a browser `PushSubscription` — base64url.
#[derive(Debug, Deserialize, ToSchema)]
pub struct PushKeys {
    pub p256dh: String,
    pub auth: String,
}

/// Body for `POST /members/:id/push-subscriptions` — the browser's
/// `PushSubscription.toJSON()` shape.
#[derive(Debug, Deserialize, ToSchema)]
pub struct RegisterPushSubscription {
    pub endpoint: String,
    pub keys: PushKeys,
}

/// Set a member's email delivery mode. An unknown `mode` fails deserialization
/// → `400`.
#[derive(Debug, Deserialize, ToSchema)]
pub struct SetDeliveryMode {
    pub mode: EmailDeliveryMode,
}

/// A member's current email delivery mode.
#[derive(Debug, Serialize, ToSchema)]
pub struct DeliveryModeView {
    pub mode: EmailDeliveryMode,
}

/// The caller's own identity: who this token/session acts as, in which
/// workspace, with what capabilities. `is_bearer` distinguishes a bearer token
/// from a session. `known_capabilities` is the full capability vocabulary,
/// so a client can render what the caller *cannot* do (vocabulary − granted) —
/// the capability card. A declared "allowed-tools" list is not a
/// grant; this is the real set.
#[derive(Debug, Serialize, ToSchema)]
pub struct WhoAmI {
    pub actor_id: uuid::Uuid,
    pub member_id: uuid::Uuid,
    pub delegation_grant_id: Option<uuid::Uuid>,
    pub workspace_id: uuid::Uuid,
    pub capabilities: Vec<String>,
    pub is_bearer: bool,
    pub known_capabilities: Vec<String>,
    /// Named sets whose full expansion the caller currently holds.
    pub capability_sets: Vec<String>,
}

/// Link a Slack channel to a Maidan thread. The link's channel and workspace
/// are derived from the thread; relayed messages are attributed to the
/// authenticated member.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LinkSlackChannel {
    pub slack_channel_id: String,
    pub thread_id: uuid::Uuid,
}

/// Bless an external destination for egress. `selector` must be an **id**: a
/// Slack channel id (`C…`/`G…`), or a GitHub repository `owner/name` — not a
/// `#channel-name`, and not `owner/name#123`. A name is mutable, so an
/// allowlist keyed on one is not an allowlist; and on GitHub the operator
/// blesses the repository, since per-issue blessing would mean a ticket per PR.
#[derive(Debug, Deserialize, ToSchema)]
pub struct AllowEgressTarget {
    pub surface: EgressSurface,
    pub selector: String,
}

/// Link a GitHub issue/PR to a Maidan thread. `repo` is the `owner/name` full
/// name; `channel_id`/`workspace_id` are derived from the thread.
#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct LinkGithubIssue {
    pub repo: String,
    pub issue_number: i64,
    pub thread_id: uuid::Uuid,
}

/// Query for `DELETE /workspaces/:wid/github-links` — `repo` carries a slash
/// (`owner/name`), so the target is a query pair rather than a path. Both
/// fields are required by the handler; they are `Option` only so a request that
/// omits them fails the capability check (403) rather than query extraction
/// (400).
#[derive(Debug, Deserialize, ToSchema, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct UnlinkGithubQuery {
    pub repo: Option<String>,
    pub issue_number: Option<i64>,
}

#[derive(Debug, Deserialize, ToSchema, IntoParams)]
#[into_params(parameter_in = Query)]
#[serde(deny_unknown_fields)]
pub struct UploadArtifactQuery {
    pub kind: ArtifactKind,
    pub mime_type: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MultipartUploadResponse {
    pub upload_id: String,
    pub object_key: String,
}

#[derive(Debug, Deserialize, ToSchema, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct MultipartUploadQuery {
    pub object_key: String,
}

#[derive(Debug, Deserialize, ToSchema, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct AbortMultipartQuery {
    pub upload_id: String,
    pub object_key: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MultipartPartResponse {
    pub part_number: i32,
    pub etag: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct MultipartPartInput {
    pub part_number: i32,
    pub etag: String,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CompleteMultipartArtifact {
    pub object_key: String,
    pub parts: Vec<MultipartPartInput>,
    pub kind: ArtifactKind,
    pub mime_type: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MintWebhookResponse {
    pub webhook: WebhookResponse,
    pub secret: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateSlashCommand {
    pub name: String,
    pub description: Option<String>,
    pub handler_kind: String,
    pub handler_target: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SlashCommandResponse {
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

impl From<maidan_types::SlashCommand> for SlashCommandResponse {
    fn from(c: maidan_types::SlashCommand) -> Self {
        Self {
            id: c.id,
            workspace_id: c.workspace_id,
            name: c.name,
            description: c.description,
            handler_kind: c.handler_kind,
            handler_target: c.handler_target,
            enabled: c.enabled,
            created_at: c.created_at,
            revoked_at: c.revoked_at,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MintSlashCommandResponse {
    pub command: SlashCommandResponse,
    pub secret: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateFsmHook {
    pub label: Option<String>,
    pub from_state: Option<String>,
    pub to_state: Option<String>,
    pub handler_kind: String,
    pub handler_target: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct FsmHookResponse {
    pub id: FsmHookId,
    pub workspace_id: WorkspaceId,
    pub label: Option<String>,
    pub from_state: Option<String>,
    pub to_state: Option<String>,
    pub handler_kind: SlashHandlerKind,
    pub handler_target: String,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

impl From<maidan_types::FsmHook> for FsmHookResponse {
    fn from(h: maidan_types::FsmHook) -> Self {
        Self {
            id: h.id,
            workspace_id: h.workspace_id,
            label: h.label,
            from_state: h.from_state.map(|s| s.as_str().to_string()),
            to_state: h.to_state.map(|s| s.as_str().to_string()),
            handler_kind: h.handler_kind,
            handler_target: h.handler_target,
            enabled: h.enabled,
            created_at: h.created_at,
            revoked_at: h.revoked_at,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MintFsmHookResponse {
    pub hook: FsmHookResponse,
    pub secret: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateWebhook {
    pub url: String,
    pub label: Option<String>,
    pub event_kinds: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct WebhookResponse {
    pub id: WebhookSubscriptionId,
    pub workspace_id: WorkspaceId,
    pub url: String,
    pub label: Option<String>,
    pub event_kinds: Vec<String>,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

impl From<maidan_types::WebhookSubscription> for WebhookResponse {
    fn from(w: maidan_types::WebhookSubscription) -> Self {
        Self {
            id: w.id,
            workspace_id: w.workspace_id,
            url: w.url,
            label: w.label,
            event_kinds: w.event_kinds,
            enabled: w.enabled,
            created_at: w.created_at,
            revoked_at: w.revoked_at,
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RegisterApp {
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AppResponse {
    pub id: AppId,
    pub workspace_id: WorkspaceId,
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub created_by: MemberId,
    pub created_at: DateTime<Utc>,
}

impl From<maidan_types::App> for AppResponse {
    fn from(a: maidan_types::App) -> Self {
        Self {
            id: a.id,
            workspace_id: a.workspace_id,
            slug: a.slug,
            name: a.name,
            description: a.description,
            created_by: a.created_by,
            created_at: a.created_at,
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct InstallApp {
    #[serde(default)]
    pub granted_capabilities: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AppInstallationResponse {
    pub id: AppInstallationId,
    pub app_id: AppId,
    pub workspace_id: WorkspaceId,
    pub bot_member_id: MemberId,
    pub granted_capabilities: Vec<String>,
    pub installed_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
}

impl From<maidan_types::AppInstallation> for AppInstallationResponse {
    fn from(i: maidan_types::AppInstallation) -> Self {
        Self {
            id: i.id,
            app_id: i.app_id,
            workspace_id: i.workspace_id,
            bot_member_id: i.bot_member_id,
            granted_capabilities: i.granted_capabilities,
            installed_at: i.installed_at,
            revoked_at: i.revoked_at,
        }
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct MintAppToken {
    pub label: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    pub expires_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub quotas: Vec<TokenQuota>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MintAppTokenResponse {
    pub id: ApiTokenId,
    pub secret: String,
    pub workspace_id: WorkspaceId,
    pub app_installation_id: AppInstallationId,
    pub bot_member_id: MemberId,
    pub capabilities: Vec<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub quotas: Vec<TokenQuota>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct MintApiToken {
    pub label: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// Named set (`maidan.agent.worker` / `maidan.human.admin`). Combined
    /// with `capabilities` this is progressive grant: requested ⊆ set.
    #[serde(default)]
    pub capability_set: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub quotas: Vec<TokenQuota>,
}

/// Holder-side attenuation body. No `token:admin` — the caller can only drop
/// rights they already hold.
#[derive(Debug, Deserialize, ToSchema)]
pub struct AttenuateToken {
    pub capabilities: Vec<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub label: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct DelegateToken {
    pub grant_id: uuid::Uuid,
    /// Optional further attenuation. Empty uses the intersection of the
    /// grant and the delegate's current authority.
    #[serde(default)]
    pub capabilities: Vec<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub label: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateDelegationGrant {
    pub subject_id: uuid::Uuid,
    pub delegate_id: uuid::Uuid,
    pub capabilities: Vec<String>,
    pub purpose: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CapabilitySetView {
    pub name: String,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SetWorkspaceHandle {
    pub handle: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreatePeer {
    pub name: String,
    pub base_url: String,
    /// Workspace on the remote peer to poll; defaults to the path workspace when omitted.
    pub remote_workspace_id: Option<uuid::Uuid>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PeerResponse {
    pub id: PeerId,
    pub workspace_id: WorkspaceId,
    pub remote_workspace_id: WorkspaceId,
    pub name: String,
    pub base_url: String,
    pub enabled: bool,
    pub last_synced_event_id: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl From<maidan_types::Peer> for PeerResponse {
    fn from(p: maidan_types::Peer) -> Self {
        Self {
            id: p.id,
            workspace_id: p.workspace_id,
            remote_workspace_id: p.remote_workspace_id,
            name: p.name,
            base_url: p.base_url,
            enabled: p.enabled,
            last_synced_event_id: p.last_synced_event_id,
            created_at: p.created_at,
            updated_at: p.updated_at,
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MintPeerResponse {
    pub peer: PeerResponse,
    pub secret: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ApiTokenSummary {
    pub id: ApiTokenId,
    pub workspace_id: WorkspaceId,
    pub member_id: MemberId,
    pub label: Option<String>,
    pub capabilities: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub expires_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SetMentionWebhook {
    pub webhook_id: Option<uuid::Uuid>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MentionWebhookConfig {
    pub webhook_id: Option<WebhookSubscriptionId>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MintApiTokenResponse {
    pub id: ApiTokenId,
    pub secret: String,
    pub workspace_id: WorkspaceId,
    pub member_id: MemberId,
    pub capabilities: Vec<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub quotas: Vec<TokenQuota>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct DelegateTokenResponse {
    pub grant_id: DelegationGrantId,
    pub delegate_id: MemberId,
    pub token: MintApiTokenResponse,
}

#[derive(Debug, Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CreateShareTicket {
    pub channel_id: uuid::Uuid,
    pub expires_at: DateTime<Utc>,
    #[serde(default)]
    pub artifact_shas: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ShareTicketResponse {
    pub ticket: ShareTicket,
    pub artifact_shas: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MintShareTicketResponse {
    pub ticket: ShareTicket,
    pub artifact_shas: Vec<String>,
    /// Returned once. Only its SHA-256 hash is persisted.
    pub secret: String,
}

#[derive(Debug, Deserialize, ToSchema, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct OidcLoginQuery {
    pub workspace_id: uuid::Uuid,
    pub return_to: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct OidcCallbackQuery {
    pub state: String,
    pub code: Option<String>,
    pub mock_sub: Option<String>,
    pub mock_email: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SessionResponse {
    pub member_id: MemberId,
    pub workspace_id: WorkspaceId,
    pub expires_at: DateTime<Utc>,
}

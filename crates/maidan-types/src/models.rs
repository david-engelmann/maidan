//! Domain models. Each `<X>` has a paired `New<X>` for inserts so the
//! caller can build state-less values without populating server-assigned
//! fields (id, timestamps).

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

    /// A terminal state — no further transitions, so a task in it counts as done
    /// for dependency readiness (Cluster 217).
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
    /// A frozen, content-addressed context pack (Cluster 329) — tamper-evident
    /// "exactly what the agent was handed".
    ContextSnapshot,
}

impl ArtifactKind {
    pub fn as_str(self) -> &'static str {
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

/// A member's role within a channel (Cluster 159). `Admin` may manage
/// membership; both roles grant access to a private channel.
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

/// Membership row for a channel (Cluster 159). Rows exist for private
/// channels; public channels are open to the whole workspace without rows.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ChannelMember {
    pub channel_id: ChannelId,
    pub member_id: MemberId,
    pub role: ChannelMemberRole,
    pub created_at: DateTime<Utc>,
}

/// A free-form skill tag a member (agent) declares (Cluster 230). Skill routing
/// matches a task's required skills against a member's declared skills.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MemberSkill {
    pub member_id: MemberId,
    pub skill: String,
    pub created_at: DateTime<Utc>,
}

/// A skill a task (thread) requires (Cluster 231). A task is claimable by a
/// member only if every required skill is one the member has declared.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ThreadRequiredSkill {
    pub thread_id: ThreadId,
    pub skill: String,
    pub created_at: DateTime<Utc>,
}

/// The structured result an agent attaches to a task when it's done (Cluster
/// 234). One per thread (a re-set overwrites). A requester — or a parent task
/// that depends on it — reads this back; coordination waits block on it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ThreadResult {
    pub thread_id: ThreadId,
    #[cfg_attr(feature = "openapi", schema(value_type = Object))]
    pub result: serde_json::Value,
    pub produced_by: MemberId,
    pub produced_at: DateTime<Utc>,
}

/// A per-thread budget envelope (Cluster 358, T1/T5). An orchestrator sets any of
/// the optional maxima; an agent reports incremental usage as it works, and when
/// a dimension is exceeded the run is stopped (the claim fails → DLQ). USD is
/// integer micros ($1 = 1_000_000) to keep money out of floats. Wall time is not
/// stored — it derives from the thread's Cluster-351 working clock
/// (`work_started_at`) against `max_wall_secs`.
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
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// The maxima an orchestrator sets on a thread's budget (Cluster 358). Each
/// dimension is optional — set the ones you want to bind; omit (or `None`) leaves
/// that dimension unbounded. Does not touch accumulated usage.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
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

/// An increment of resource usage an agent reports against a thread's budget
/// (Cluster 358). Each dimension defaults to 0.
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

/// The outcome of reporting usage against a thread's budget (Cluster 358). Always
/// carries the new totals; `stopped` is true when this report pushed the thread
/// over budget and its claimed run was stopped (claim released + `ClaimFailed` +
/// DLQ), with `reason` the dimension that bound.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct UsageReport {
    pub budget: ThreadBudget,
    pub stopped: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Which budget dimension was exceeded (Cluster 358) — the reason a run was
/// stopped, carried on the `ClaimFailed` event and the DLQ entry.
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
    /// (tokens, usd, turns, wall). `wall_secs_elapsed` is the thread's working-clock
    /// elapsed time (Cluster 351); pass `None` when the thread isn't working (the
    /// wall dimension is then never exceeded). A dimension with no maximum, or a
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
        if let (Some(max), Some(elapsed)) = (self.max_wall_secs, wall_secs_elapsed) {
            if max > 0 && elapsed >= max {
                return Some(BudgetReason::Wall);
            }
        }
        None
    }
}

/// A member's notifications for one thread, collapsed (Cluster 359, N5). The
/// grouped inbox shows one row per thread — the newest notification plus how many
/// (and how many unread) it stands for — so a busy thread doesn't flood the flat
/// list. `thread_id` is `None` for the group of notifications that carry no thread.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct NotificationThreadGroup {
    pub thread_id: Option<ThreadId>,
    pub count: i64,
    pub unread_count: i64,
    /// The newest notification in the group (its `created_at` orders the groups).
    pub latest: Notification,
}

/// Collapse a member's notifications into per-thread groups (Cluster 359, N5),
/// newest-activity first. Each group's `latest` is its most recent notification;
/// groups are ordered by that notification's `created_at` (descending). The input
/// is assumed newest-first (as [`Notification`] lists are), so the first
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

/// A dead-lettered agent run (Cluster 358, T1/T5). When a claimed run is stopped
/// because it exceeded its budget envelope, the claim fails and a DLQ entry is
/// recorded — so the failed work is triageable (retry, raise the budget, give up)
/// rather than silently lost or silently marked done. Captures the failure
/// snapshot: which thread, which agent, why, and usage at the moment of failure.
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

/// A new dead-letter entry to record (Cluster 358). `id`/`failed_at` are assigned
/// by the store.
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

/// Persisted steering guidance for a task/thread (Cluster 355, W1). A durable
/// instruction from the owner (or a supervisor) that survives claims and
/// handoffs, so a resuming or newly-assigned agent reads the CURRENT steer. One
/// per thread (a re-set overwrites). Distinct from a Cluster-195 handoff note,
/// which rides an assignment event and is not persisted.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ThreadSteer {
    pub thread_id: ThreadId,
    pub steer: String,
    pub steered_by: MemberId,
    pub steered_at: DateTime<Utc>,
}

/// The state of an approval gate (Cluster 350, the held gate). A gate opens
/// `Pending`; a human resolves it to exactly one of accept/decline/cancel.
/// Silence never resolves a gate (there is no timeout auto-approve), and a
/// resolve is a compare-and-set on `Pending` so a double-answer can't flip it.
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

/// A durable, queryable human-approval gate (Cluster 350, the held gate). An
/// agent's `request_approval` opens one `Pending` gate and returns an
/// `input-required` result instead of blocking; a human later resolves it via
/// the `/ui`. Persisted so the gate survives a dropped connection and can be
/// listed while outstanding (queryable). An optional `thread_id` attaches the
/// gate to a thread for the N6 required-human claim gate.
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
    pub created_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
}

/// The inputs to open a new approval gate (Cluster 350).
#[derive(Debug, Clone)]
pub struct NewApprovalGate {
    pub workspace_id: WorkspaceId,
    pub thread_id: Option<ThreadId>,
    pub requested_by: MemberId,
    pub prompt: String,
    pub schema: Option<serde_json::Value>,
}

/// A per-recipient notification (Cluster 237, Program C). Where a mention is one
/// shared `maidan_mentions` row read through a single inbox cursor, this is one
/// row per (recipient, source event): *who* should know, *what* triggered it
/// (`kind` = the source [`EventKind`] + `source_log_id` = the event-log row),
/// denormalized context (`channel/thread/message/actor`) so the inbox renders
/// without re-fetching the event, and per-recipient read state. The
/// zero-blast-radius foundation for the notification router + unified inbox that
/// follow — nothing writes rows yet.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Notification {
    pub id: NotificationId,
    pub workspace_id: WorkspaceId,
    /// The recipient.
    pub member_id: MemberId,
    pub kind: crate::EventKind,
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
    /// Snoozed until this instant (Cluster 359, N5) — while in the future the
    /// notification is hidden from the default inbox + badge, then resurfaces.
    /// `None` = not snoozed. Orthogonal to `read_at`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub snoozed_until: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct NewNotification {
    pub workspace_id: WorkspaceId,
    pub member_id: MemberId,
    pub kind: crate::EventKind,
    pub source_log_id: i64,
    pub channel_id: Option<ChannelId>,
    pub thread_id: Option<ThreadId>,
    pub message_id: Option<MessageId>,
    pub actor_id: Option<MemberId>,
}

/// A member's notification preference for one event kind (Cluster 241, Program C
/// Arc H). `muted` suppresses router-written notifications of `kind` for this
/// member; the absence of a row is the default (notify). The routing brain the
/// notification router consults before writing.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct NotificationPref {
    pub member_id: MemberId,
    pub kind: crate::EventKind,
    pub muted: bool,
    pub updated_at: DateTime<Utc>,
}

/// A member following a channel (Cluster 244, Arc H) — presence = following. The
/// notification router notifies followers of activity in the channel, honoring mutes.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ChannelFollow {
    pub member_id: MemberId,
    pub channel_id: ChannelId,
    pub created_at: DateTime<Utc>,
}

/// A member following a thread (Cluster 244, Arc H) — presence = following.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ThreadFollow {
    pub member_id: MemberId,
    pub thread_id: ThreadId,
    pub created_at: DateTime<Utc>,
}

/// A workspace's content graph for import (Cluster 269) — the flat, id-linked
/// collections of an export bundle, ready to insert. The server flattens its
/// `WorkspaceExport` (which nests channel members under each channel) into this and
/// optionally remaps every id for a fresh-workspace import.
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

/// A member's delivery email address (Cluster 248, Arc I) — where email
/// notifications go. One per member.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct MemberEmail {
    pub member_id: MemberId,
    pub email: String,
    pub updated_at: DateTime<Utc>,
}

/// A claimed entry from the durable mail outbox (Cluster 304) the retry worker
/// will attempt to send. `attempts` includes the current claim. Content-only —
/// the outbox's status / scheduling columns stay internal to the store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MailOutbox {
    pub id: MailOutboxId,
    pub to_address: String,
    pub subject: String,
    pub body: String,
    pub attempts: i64,
}

/// A new outbound notification email to enqueue for durable, retryable delivery
/// (Cluster 304). Enqueued `pending` with `next_attempt_at = now`.
#[derive(Debug, Clone)]
pub struct NewMailOutbox {
    pub to_address: String,
    pub subject: String,
    pub body: String,
}

/// A dead-lettered outbox entry for the operator DLQ view (Cluster 306): a message
/// that exhausted its retries. `last_error` is why the final attempt failed.
#[derive(Debug, Clone, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct DeadMail {
    pub id: MailOutboxId,
    pub to_address: String,
    pub subject: String,
    pub attempts: i64,
    pub last_error: Option<String>,
    pub updated_at: DateTime<Utc>,
}

/// A Slack projector channel link (Cluster 308): a Slack channel projects into the
/// `thread_id` in `channel_id`/`workspace_id`, with inbound Slack messages posted as
/// `member_id`. One Maidan thread per Slack channel.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct SlackChannelLink {
    pub slack_channel_id: String,
    pub workspace_id: WorkspaceId,
    pub channel_id: ChannelId,
    pub thread_id: ThreadId,
    pub member_id: MemberId,
    pub created_at: DateTime<Utc>,
}

/// A new Slack channel link to create (Cluster 308).
#[derive(Debug, Clone)]
pub struct NewSlackChannelLink {
    pub slack_channel_id: String,
    pub workspace_id: WorkspaceId,
    pub channel_id: ChannelId,
    pub thread_id: ThreadId,
    pub member_id: MemberId,
}

/// A GitHub projector issue/PR link (Cluster 311): a GitHub issue/PR (`repo`
/// full-name + `issue_number`) projects into the `thread_id` in
/// `channel_id`/`workspace_id`, with inbound comments posted as `member_id`. One
/// Maidan thread per GitHub issue/PR.
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
}

/// A new GitHub issue/PR link to create (Cluster 311).
#[derive(Debug, Clone)]
pub struct NewGithubIssueLink {
    pub repo: String,
    pub issue_number: i64,
    pub workspace_id: WorkspaceId,
    pub channel_id: ChannelId,
    pub thread_id: ThreadId,
    pub member_id: MemberId,
}

/// How a member wants notification emails delivered (Cluster 254, Arc I). The
/// default (an absent preference row) is `Immediate` — the Cluster-249 behaviour.
/// `Digest` opts out of per-notification emails in favour of a periodic rollup
/// from the digest sweeper; the two are mutually exclusive by design.
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

/// A member due for an email digest (Cluster 254, Arc I): the sweeper's enumeration
/// row — a digest-mode member with an address who has unread notifications created
/// since their last digest. Carries the address so the sweeper needs no extra
/// per-member lookup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DigestDue {
    pub member_id: MemberId,
    pub email: String,
    pub unread_count: i64,
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
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct PostDmMessage {
    pub author_id: uuid::Uuid,
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
    pub state: ThreadState,
    /// The member this thread/task is assigned to, if any (Cluster 171). An
    /// axis orthogonal to [`ThreadState`]: assignment persists across state
    /// transitions. Set via assign/handoff, atomic claim, or cleared on unassign.
    pub assignee_id: Option<MemberId>,
    /// Lease deadline for a claimed assignment (Cluster 192). When set and in the
    /// past, the assignment is reclaimable by the next `claim_next` (dead-agent
    /// recovery); `None` is a durable assignment with no lease.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignment_expires_at: Option<DateTime<Utc>>,
    /// The fencing value for the current claim (Cluster 351, the occupancy
    /// clocks). A fresh resource-version minted every time `assignee_id` is set
    /// (claim / claim_next / assign) and cleared on unassign. `renew_claim` and
    /// other claim-holder operations must present the matching value — a TTL
    /// lease alone lets a stale holder act after the next owner has taken over.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claim_lease_id: Option<ClaimLeaseId>,
    /// The working clock (Cluster 351). `assignment_expires_at` is the *claim*
    /// clock (lease deadline); this is when the current holder acknowledged and
    /// began work (`acknowledge_claim`). `None` = claimed but not yet started, or
    /// unassigned. Reset to `None` on every (re)claim/assign/unassign so it always
    /// reflects the CURRENT claim epoch — letting occupancy separate a
    /// claimed-but-idle agent from one actively working.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub work_started_at: Option<DateTime<Utc>>,
    /// The durable OWNER of this task/thread (Cluster 355, W1): the accountable
    /// party — a human, typically — distinct from the [`Thread::assignee_id`]
    /// claimer that does the work. Orthogonal to the FSM and the claim axis. The
    /// owner receives stuck notifications and, once set, opts the thread into
    /// separation-of-duties (the claimer cannot land its own work). `None` = no
    /// designated owner (unrestricted).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_id: Option<MemberId>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub tombstoned_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct NewThread {
    pub channel_id: ChannelId,
    pub parent_thread_id: Option<ThreadId>,
    pub title: Option<String>,
}

/// A child thread collapsed under its parent (Cluster 356, F2): the child thread
/// plus a live count of its (non-tombstoned) messages, so a threaded view can show
/// "N replies" without loading each child's messages.
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

/// Outcome of an atomic [`Thread`] claim (Cluster 171): `claimed` is `true` when
/// this call won the compare-and-set (the thread was unassigned and is now the
/// caller's), `false` when it was already assigned. `thread` is the current row
/// either way.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ThreadClaimResult {
    pub thread: Thread,
    pub claimed: bool,
}

/// A channel's task-queue depth (Cluster 224) — a point-in-time partition of its
/// **open** (non-terminal, non-tombstoned) task threads, for an orchestrator
/// deciding whether to scale workers. The three sub-counts partition `open`:
/// - `assigned`: actively held (an assignee with a live, non-expired lease).
/// - `ready`: claimable now — unassigned or lease-expired, and every dependency
///   terminal (the `claim_next` predicate).
/// - `blocked`: unassigned/lease-expired but waiting on a non-terminal dependency.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct QueueDepth {
    pub open: i64,
    pub ready: i64,
    pub assigned: i64,
    pub blocked: i64,
}

/// The occupancy of a channel's **open** task threads (Cluster 351) — the
/// two-clocks refinement of [`QueueDepth`]. It splits `assigned` by the *working*
/// clock, so an orchestrator sees not just how much work is held but how much is
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

/// A schedule that materializes a task thread when due (Cluster 226). A one-shot
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
}

#[derive(Debug, Clone)]
pub struct NewTaskSchedule {
    pub workspace_id: WorkspaceId,
    pub channel_id: ChannelId,
    pub title: String,
    pub interval_secs: Option<i64>,
    pub next_run_at: DateTime<Utc>,
    pub created_by: MemberId,
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

/// A task-dependency DAG edge (Cluster 217): the task `thread_id` depends on
/// `depends_on_thread_id` — i.e. it is blocked until that dependency reaches a
/// terminal state. Edges are directed; the pair is unique.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ThreadDependency {
    pub thread_id: ThreadId,
    pub depends_on_thread_id: ThreadId,
    pub created_at: DateTime<Utc>,
}

/// A typed part of a message's structured content (Cluster 173). The wire form
/// is internally tagged (`{"type":"text","text":"…"}`), matching the MCP /
/// Anthropic content-block dialect and the existing A2A `TextPart`. `body`
/// remains the canonical searchable plain-text projection derived from these.
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

/// Derive the plain-text `body` projection from structured content blocks
/// (Cluster 173) so full-text + semantic search stay unchanged. `ToolUse` adds
/// nothing (a tool name is not prose); code is fenced; a resource link renders
/// as its title or URI. Blocks are joined by blank lines.
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
/// `artifact_sha256` / `sha256` scalar fields plus an `artifacts` array of either
/// bare SHA strings or `{sha256}` objects. Sorted + deduped. Shared by the REST
/// and MCP context assemblers (Cluster 335) so both surface the same artifacts.
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

/// One tool invocation in a thread's transcript (Cluster 197): a
/// [`ContentBlock::ToolUse`] paired with its [`ContentBlock::ToolResult`]
/// (correlated by id), plus the message context each block came from.
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
/// messages (Cluster 197) — surfaced rather than dropped so a gap is visible.
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

/// A thread's tool-call transcript (Cluster 197): every [`ContentBlock::ToolUse`]
/// across the thread's messages, each correlated with its `ToolResult` by id,
/// plus any results whose call is outside the scanned window. A token-lean
/// projection of the tool structure — `Text`/`Code`/`ResourceLink` blocks and
/// `body` are dropped.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct ToolTranscript {
    pub thread_id: ThreadId,
    pub entries: Vec<ToolCallEntry>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub orphan_results: Vec<OrphanToolResult>,
}

/// Extract a [`ToolTranscript`] from a thread's messages (Cluster 197). Walks
/// each non-tombstoned message's structured content, pairing every `ToolUse`
/// with the first `ToolResult` carrying the same id (correlation is
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
/// object — used to omit an empty `metadata` from the wire (Cluster 177).
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
    /// Open annotation bag. Omitted from the wire when empty (Cluster 177, token
    /// round 3) — most messages carry no metadata, so `"metadata":{}` on every
    /// one was pure token waste. Deserializes back to an empty object by default.
    #[serde(skip_serializing_if = "json_value_is_empty", default)]
    pub metadata: serde_json::Value,
    /// Typed structured content (Cluster 173); `None` for plain/legacy messages.
    /// `body` is the plain-text projection of these blocks.
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

/// Body/metadata replacement for [`Store::edit_message`] (Cluster 29).
#[derive(Debug, Clone)]
pub struct EditMessage {
    pub body: String,
    pub metadata: serde_json::Value,
    pub content: Option<Vec<ContentBlock>>,
}

/// One recorded body change for a message (Cluster 46).
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct Vote {
    pub message_id: MessageId,
    pub member_id: MemberId,
    pub kind: String,
    /// Optional confidence weight (Cluster 324), by convention in `0..=1`, for
    /// weighted consensus. `None` when the voter stated no confidence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewVote {
    pub message_id: MessageId,
    pub member_id: MemberId,
    pub kind: String,
    /// Optional confidence weight (Cluster 324), by convention `0..=1`.
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

/// The typed predicate on a [`Reference`] edge (Cluster 319). A small controlled
/// vocabulary — the same subject→predicate→object shape as IBIS, W3C PROV,
/// ClaimReview, and GitHub/Linear issue relations — so an agent's edges are
/// machine-navigable ("what `refutes` this", "what this `supersedes`") instead of
/// free prose. [`Other`] keeps expressivity: an unrecognized relation round-trips
/// verbatim rather than being rejected. Serializes as the bare snake_case string on
/// the wire (a controlled variant → its canonical name; `Other(s)` → `s`).
///
/// [`Other`]: RelationKind::Other
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
    /// This entity was seeded/branched from the target (re-ask lineage, Cluster
    /// 327): a new work thread spawned from a source message.
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
    /// The typed predicate (Cluster 319). Wire form is a snake_case string; the
    /// controlled set is [`RelationKind::CONTROLLED`], unknown values round-trip via
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

/// A workspace's canonical definition of a term (Cluster 321) — the anti-drift pin
/// so agents use words the same way, and the target of the `defines` reference
/// relation. One entry per `(workspace_id, term)`. Flat by design (no hierarchy).
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
    pub kind: ArtifactKind,
    pub uploaded_by: Option<MemberId>,
    pub created_at: DateTime<Utc>,
    pub tombstoned_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct NewArtifact {
    pub sha256: String,
    pub size_bytes: i64,
    pub mime_type: Option<String>,
    pub kind: ArtifactKind,
    pub uploaded_by: Option<MemberId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEvent {
    pub id: i64,
    pub occurred_at: DateTime<Utc>,
    pub actor_id: Option<MemberId>,
    pub action: String,
    pub target_kind: Option<String>,
    pub target_id: Option<uuid::Uuid>,
    pub metadata: serde_json::Value,
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

/// A one-time OAuth authorization code persisted for cross-replica exchange
/// (Cluster 104). Only the SHA-256 hash of the code is stored.
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
    pub csrf_secret: String,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewMaidanSession {
    pub workspace_id: WorkspaceId,
    pub member_id: MemberId,
    pub csrf_secret: String,
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

#[derive(Debug, Clone)]
pub struct NewAuditEvent {
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
}

impl SlashHandlerKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::McpTool => "mcp_tool",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "http" => Some(Self::Http),
            "mcp_tool" => Some(Self::McpTool),
            _ => None,
        }
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
/// and survives restart (Cluster 104). `job_id`/`workspace_id` are raw UUIDs to
/// match the operator HTTP shape.
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
        // {} and null both carry no info → omitted (Cluster 177).
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

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
}

impl ArtifactKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Screenshot => "screenshot",
            Self::Recording => "recording",
            Self::Transcript => "transcript",
            Self::CodeDump => "code_dump",
            Self::Attachment => "attachment",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "screenshot" => Some(Self::Screenshot),
            "recording" => Some(Self::Recording),
            "transcript" => Some(Self::Transcript),
            "code_dump" => Some(Self::CodeDump),
            "attachment" => Some(Self::Attachment),
            _ => None,
        }
    }

    pub fn default_mime(self) -> &'static str {
        match self {
            Self::Screenshot => "image/png",
            Self::Recording | Self::Attachment => "application/octet-stream",
            Self::Transcript | Self::CodeDump => "text/plain",
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

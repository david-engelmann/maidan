//! Event taxonomy emitted by every state-changing operation.
//!
//! Events are externally tagged so wire-format consumers can switch on
//! the `kind` field. Filters select a subset of the stream by workspace
//! / channel / thread / member / kind without touching the payload.

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::*;
use crate::models::*;
use crate::PayerStamp;

/// Row in the persistent `maidan_events` log (Cluster D.6).
///
/// Wire JSON (REST `GET /events`, federation pull, A2A envelopes) also
/// carries `$type` (`maidan.event.{kind}/1`) via [`crate::lexicon::stored_event_wire`].
/// That field is **not** a `maidan_events` column — the stored tag remains
/// `kind`. Deserialize ignores unknown fields, including `$type`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct StoredEvent {
    pub id: i64,
    /// Event-log position of this row. Equal to [`StoredEvent::id`]. **Not** a
    /// Postgres WAL [`crate::Lsn`] / `Maidan-Consistency-Token`.
    #[serde(default)]
    pub lsn: i64,
    pub kind: EventKind,
    pub workspace_id: Option<WorkspaceId>,
    pub channel_id: Option<ChannelId>,
    pub thread_id: Option<ThreadId>,
    pub payload: serde_json::Value,
    pub occurred_at: DateTime<Utc>,
    /// SHA-256 commitment of the previous event in this workspace, or genesis.
    #[serde(default)]
    pub prev_hash: String,
    /// SHA-256 of canonical JSON of [`StoredEvent::payload`].
    #[serde(default)]
    pub content_hash: String,
    /// The key that opens this event's sealed words, while they are live
    /// ([`crate::content_seal`]). `None` when nothing is sealed or the words
    /// were shredded. `StoredEvent`'s own `Serialize` never writes it; a
    /// whole-log reader gets it through [`crate::KeyedEvent`].
    #[serde(default)]
    pub content_key: Option<crate::ContentKey>,
    /// The server span this event was written under. Not part of the content
    /// hash. Absent when no request carried a trace into the write.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace: Option<crate::TraceContext>,
}

impl StoredEvent {
    /// Who wrote this event and on whose behalf, read from the payload where it
    /// is stored under the event hash. `None` for background work and for events
    /// written before attribution existed.
    pub fn attribution(&self) -> Option<crate::Attribution> {
        self.payload
            .get("attribution")
            .and_then(|value| serde_json::from_value(value.clone()).ok())
    }

    /// The payload with its words restored where the key is live. A shredded
    /// event comes back as stored: its `sealed` block and an empty body.
    pub fn opened_payload(&self) -> Result<serde_json::Value, crate::SealError> {
        let mut payload = self.payload.clone();
        crate::open_payload(&mut payload, self.content_key.as_ref())?;
        Ok(payload)
    }

    /// Open the payload in place and drop the key: the event as a reader who
    /// is not handed keys sees it. The content hash no longer matches an
    /// opened payload; only a sealed one verifies.
    pub fn open(&mut self) -> Result<(), crate::SealError> {
        crate::open_payload(&mut self.payload, self.content_key.as_ref())?;
        self.content_key = None;
        Ok(())
    }

    /// The event, its words opened when the key is live.
    pub fn opened_event(&self) -> Result<Event, OpenEventError> {
        Ok(serde_json::from_value(self.opened_payload()?)?)
    }

    /// Whether this event's words were sealed and their key is gone.
    pub fn is_shredded(&self) -> bool {
        self.payload.get("sealed").is_some() && self.content_key.is_none()
    }

    /// Chain fields a peer verifies without trusting the host.
    pub fn link(&self) -> crate::event_chain::EventLink {
        crate::event_chain::EventLink {
            id: self.id,
            lsn: self.lsn,
            prev_hash: self.prev_hash.clone(),
            content_hash: self.content_hash.clone(),
        }
    }
}

/// OpenAPI / utoipa shape of [`StoredEvent`]: the durable columns plus the
/// wire-only `$type` this type still defers.
#[cfg(feature = "openapi")]
#[allow(dead_code)]
#[derive(utoipa::ToSchema)]
struct StoredEventOpenApi {
    /// Observable lexicon type. Computed from `kind` on serialize. Not stored.
    #[schema(rename = "$type", example = "maidan.event.message_posted/1")]
    r#type: String,
    id: i64,
    lsn: i64,
    kind: EventKind,
    workspace_id: Option<WorkspaceId>,
    channel_id: Option<ChannelId>,
    thread_id: Option<ThreadId>,
    payload: serde_json::Value,
    occurred_at: DateTime<Utc>,
    prev_hash: String,
    content_hash: String,
    /// Base64 key that opens `payload.sealed`. Only on whole-log reads (a
    /// federation peer, catch-up) and only while the words are live. A
    /// `sealed` block without it means the words were shredded.
    #[schema(nullable = false)]
    content_key: Option<String>,
    /// W3C `traceparent` of the server span that wrote the event. Not hashed.
    #[schema(nullable = true)]
    traceparent: Option<String>,
    /// W3C `tracestate` that travelled with `traceparent`.
    #[schema(nullable = true)]
    tracestate: Option<String>,
}

#[cfg(feature = "openapi")]
impl utoipa::PartialSchema for StoredEvent {
    fn schema() -> utoipa::openapi::RefOr<utoipa::openapi::schema::Schema> {
        <StoredEventOpenApi as utoipa::PartialSchema>::schema()
    }
}

#[cfg(feature = "openapi")]
impl utoipa::ToSchema for StoredEvent {
    fn name() -> std::borrow::Cow<'static, str> {
        "StoredEvent".into()
    }

    fn schemas(
        schemas: &mut Vec<(
            String,
            utoipa::openapi::RefOr<utoipa::openapi::schema::Schema>,
        )>,
    ) {
        <StoredEventOpenApi as utoipa::ToSchema>::schemas(schemas);
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    WorkspaceCreated,
    MemberJoined,
    ChannelCreated,
    ThreadCreated,
    ThreadStateChanged,
    ThreadAssignmentChanged,
    ThreadReady,
    ThreadResultSet,
    ApprovalRequested,
    ThreadBlocked,
    BlockedResolved,
    StatusDeclared,
    ClaimExpired,
    ClaimUnacknowledged,
    ClaimFailed,
    UsageReported,
    ThreadLanded,
    ReviewSubmitted,
    WaitTimedOut,
    ScheduleSkipped,
    ThreadSpawnDenied,
    ProjectorMisconfigured,
    MemberFrozen,
    MemberUnfrozen,
    MessagePosted,
    MessageEdited,
    MessageTombstoned,
    MentionRecorded,
    VoteCast,
    ReactionAdded,
    ReactionRemoved,
    MessagePinned,
    MessageUnpinned,
    ReferenceAdded,
    ArtifactUpserted,
    MemoryBlockUpdated,
}

impl EventKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WorkspaceCreated => "workspace_created",
            Self::MemberJoined => "member_joined",
            Self::ChannelCreated => "channel_created",
            Self::ThreadCreated => "thread_created",
            Self::ThreadStateChanged => "thread_state_changed",
            Self::ThreadAssignmentChanged => "thread_assignment_changed",
            Self::ThreadReady => "thread_ready",
            Self::ThreadResultSet => "thread_result_set",
            Self::ApprovalRequested => "approval_requested",
            Self::ThreadBlocked => "thread_blocked",
            Self::BlockedResolved => "blocked_resolved",
            Self::StatusDeclared => "status_declared",
            Self::ClaimExpired => "claim_expired",
            Self::ClaimUnacknowledged => "claim_unacknowledged",
            Self::ClaimFailed => "claim_failed",
            Self::UsageReported => "usage_reported",
            Self::ThreadLanded => "thread_landed",
            Self::ReviewSubmitted => "review_submitted",
            Self::WaitTimedOut => "wait_timed_out",
            Self::ScheduleSkipped => "schedule_skipped",
            Self::ThreadSpawnDenied => "thread_spawn_denied",
            Self::ProjectorMisconfigured => "projector_misconfigured",
            Self::MemberFrozen => "member_frozen",
            Self::MemberUnfrozen => "member_unfrozen",
            Self::MessagePosted => "message_posted",
            Self::MessageEdited => "message_edited",
            Self::MessageTombstoned => "message_tombstoned",
            Self::MentionRecorded => "mention_recorded",
            Self::VoteCast => "vote_cast",
            Self::ReactionAdded => "reaction_added",
            Self::ReactionRemoved => "reaction_removed",
            Self::MessagePinned => "message_pinned",
            Self::MessageUnpinned => "message_unpinned",
            Self::ReferenceAdded => "reference_added",
            Self::ArtifactUpserted => "artifact_upserted",
            Self::MemoryBlockUpdated => "memory_block_updated",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "workspace_created" => Some(Self::WorkspaceCreated),
            "member_joined" => Some(Self::MemberJoined),
            "channel_created" => Some(Self::ChannelCreated),
            "thread_created" => Some(Self::ThreadCreated),
            "thread_state_changed" => Some(Self::ThreadStateChanged),
            "thread_assignment_changed" => Some(Self::ThreadAssignmentChanged),
            "thread_ready" => Some(Self::ThreadReady),
            "thread_result_set" => Some(Self::ThreadResultSet),
            "approval_requested" => Some(Self::ApprovalRequested),
            "thread_blocked" => Some(Self::ThreadBlocked),
            "blocked_resolved" => Some(Self::BlockedResolved),
            "status_declared" => Some(Self::StatusDeclared),
            "claim_expired" => Some(Self::ClaimExpired),
            "claim_unacknowledged" => Some(Self::ClaimUnacknowledged),
            "claim_failed" => Some(Self::ClaimFailed),
            "usage_reported" => Some(Self::UsageReported),
            "thread_landed" => Some(Self::ThreadLanded),
            "review_submitted" => Some(Self::ReviewSubmitted),
            "wait_timed_out" => Some(Self::WaitTimedOut),
            "schedule_skipped" => Some(Self::ScheduleSkipped),
            "thread_spawn_denied" => Some(Self::ThreadSpawnDenied),
            "projector_misconfigured" => Some(Self::ProjectorMisconfigured),
            "member_frozen" => Some(Self::MemberFrozen),
            "member_unfrozen" => Some(Self::MemberUnfrozen),
            "message_posted" => Some(Self::MessagePosted),
            "message_edited" => Some(Self::MessageEdited),
            "message_tombstoned" => Some(Self::MessageTombstoned),
            "mention_recorded" => Some(Self::MentionRecorded),
            "vote_cast" => Some(Self::VoteCast),
            "reaction_added" => Some(Self::ReactionAdded),
            "reaction_removed" => Some(Self::ReactionRemoved),
            "message_pinned" => Some(Self::MessagePinned),
            "message_unpinned" => Some(Self::MessageUnpinned),
            "reference_added" => Some(Self::ReferenceAdded),
            "artifact_upserted" => Some(Self::ArtifactUpserted),
            "memory_block_updated" => Some(Self::MemoryBlockUpdated),
            _ => None,
        }
    }

    /// Observable `$type` for this kind. Version `/1` is the current
    /// generation. A breaking change is a **new type** (`/2`), not a field
    /// rename — the string itself is the contract (Hyrum's Law home). Stored
    /// `maidan_events.payload` still tags on `kind`; `$type` is injected on
    /// wire envelopes.
    pub fn type_id(self) -> String {
        format!("maidan.event.{}/1", self.as_str())
    }

    /// Parse `maidan.event.{kind}/1`. A different version does not match `/1`.
    pub fn parse_type_id(s: &str) -> Option<Self> {
        let rest = s.strip_prefix("maidan.event.")?;
        let (kind, version) = rest.rsplit_once('/')?;
        if version != "1" {
            return None;
        }
        Self::parse(kind)
    }

    /// Every variant, for exhaustive iteration (round-trip guards, catalogs).
    /// Kept in sync with the enum by the compile-time tripwire in
    /// `all_variants_round_trip` — a new variant fails that test's exhaustive
    /// match until it is listed here. `as_str`/`parse` are the single source of
    /// truth for the wire form; the store layer parses through `parse` so there
    /// is no per-backend copy to drift.
    pub const ALL: &'static [EventKind] = &[
        Self::WorkspaceCreated,
        Self::MemberJoined,
        Self::ChannelCreated,
        Self::ThreadCreated,
        Self::ThreadStateChanged,
        Self::ThreadAssignmentChanged,
        Self::ThreadReady,
        Self::ThreadResultSet,
        Self::ApprovalRequested,
        Self::ThreadBlocked,
        Self::BlockedResolved,
        Self::StatusDeclared,
        Self::ClaimExpired,
        Self::ClaimUnacknowledged,
        Self::ClaimFailed,
        Self::UsageReported,
        Self::ThreadLanded,
        Self::ReviewSubmitted,
        Self::WaitTimedOut,
        Self::ScheduleSkipped,
        Self::ThreadSpawnDenied,
        Self::ProjectorMisconfigured,
        Self::MemberFrozen,
        Self::MemberUnfrozen,
        Self::MessagePosted,
        Self::MessageEdited,
        Self::MessageTombstoned,
        Self::MentionRecorded,
        Self::VoteCast,
        Self::ReactionAdded,
        Self::ReactionRemoved,
        Self::MessagePinned,
        Self::MessageUnpinned,
        Self::ReferenceAdded,
        Self::ArtifactUpserted,
        Self::MemoryBlockUpdated,
    ];

    /// Whether a federated peer may push this event kind on ingest.
    /// **Allowlist-by-default via an exhaustive match**: a new event kind fails
    /// to compile here until it is consciously classified, so a peer can never
    /// inject an unreviewed kind into the local event log.
    ///
    /// All collaboration-content kinds are federatable — federation replicates
    /// the content event stream. **`ArtifactUpserted` is the exception**:
    /// federation replicates *events*, not artifact *blobs*, so an ingested
    /// `ArtifactUpserted` would announce a `sha256` whose bytes never arrive —
    /// a dangling reference. We therefore do not accept artifact-existence
    /// claims from peers.
    pub fn federatable(self) -> bool {
        match self {
            Self::WorkspaceCreated
            | Self::MemberJoined
            | Self::ChannelCreated
            | Self::ThreadCreated
            | Self::ThreadStateChanged
            | Self::ThreadAssignmentChanged
            | Self::MessagePosted
            | Self::MessageEdited
            | Self::MessageTombstoned
            | Self::MentionRecorded
            | Self::VoteCast
            | Self::ReactionAdded
            | Self::ReactionRemoved
            | Self::MessagePinned
            | Self::MessageUnpinned
            | Self::ReferenceAdded => true,
            // Blob bytes are not federated — an ingested claim would dangle.
            Self::ArtifactUpserted => false,
            // Readiness is a *locally derived* signal (this deployment's dependency
            // graph + thread states); a peer must not inject it — we compute our own.
            Self::ThreadReady => false,
            // A task result is produced locally; a peer must not inject one.
            Self::ThreadResultSet => false,
            // Approval gates are local human-control state.
            Self::ApprovalRequested => false,
            // A block is *this* deployment's dispatch decision; a peer must
            // not inject one.
            Self::ThreadBlocked => false,
            // An unblock is *this* deployment's dispatch decision; a peer must
            // not inject one.
            Self::BlockedResolved => false,
            // A status declaration is *this* deployment's agent state; a peer
            // must not inject one.
            Self::StatusDeclared => false,
            // A lease expiry is detected locally (this deployment's clock + reclaim);
            // a peer must not inject a claim of one.
            Self::ClaimExpired => false,
            // An unacknowledged claim is measured on this deployment's clock;
            // a peer must not inject one.
            Self::ClaimUnacknowledged => false,
            // A budget-exhaustion / run failure is a locally-derived signal
            // (this deployment's budget accounting); a peer must not inject one.
            Self::ClaimFailed => false,
            // Usage is an economic record derived from this deployment's
            // authenticated claim holder and price evidence.
            Self::UsageReported => false,
            // A "landed" fact is derived from *this* deployment's GitHub
            // projector webhook; a peer must not inject one for our threads.
            Self::ThreadLanded => false,
            // A verdict is this deployment's governance record: the reviews
            // table is what the close-gate reads, and a peer must not announce
            // a verdict nobody gave here.
            Self::ReviewSubmitted => false,
            // A wait timeout is fired by *this* deployment's sweeper (this
            // clock); a peer must not inject one.
            Self::WaitTimedOut => false,
            // A skipped firing is *this* deployment's scheduler decision; a
            // peer must not inject one.
            Self::ScheduleSkipped => false,
            // A refused spawn is *this* deployment's budget decision; a peer
            // must not inject one for our threads.
            Self::ThreadSpawnDenied => false,
            // A broken projector link is *this* deployment's connector
            // credentials and *this* deployment's link table; a peer has no
            // standing to declare our egress misconfigured.
            Self::ProjectorMisconfigured => false,
            // A freeze is *this* deployment's kill-switch over its own
            // members; a peer has no standing to freeze or unfreeze one.
            Self::MemberFrozen | Self::MemberUnfrozen => false,
            // A memory-block update is a locally-derived signal over local
            // shared state; a peer must not inject one.
            Self::MemoryBlockUpdated => false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    WorkspaceCreated {
        occurred_at: DateTime<Utc>,
        workspace: Workspace,
    },
    MemberJoined {
        occurred_at: DateTime<Utc>,
        workspace_id: WorkspaceId,
        member: Member,
    },
    ChannelCreated {
        occurred_at: DateTime<Utc>,
        workspace_id: WorkspaceId,
        channel: Channel,
    },
    ThreadCreated {
        occurred_at: DateTime<Utc>,
        workspace_id: WorkspaceId,
        channel_id: ChannelId,
        thread: Thread,
    },
    ThreadStateChanged {
        occurred_at: DateTime<Utc>,
        workspace_id: WorkspaceId,
        channel_id: ChannelId,
        thread_id: ThreadId,
        actor_id: MemberId,
        from_state: ThreadState,
        to_state: ThreadState,
        thread: Thread,
    },
    ThreadAssignmentChanged {
        occurred_at: DateTime<Utc>,
        workspace_id: WorkspaceId,
        channel_id: ChannelId,
        thread_id: ThreadId,
        actor_id: MemberId,
        previous_assignee_id: Option<MemberId>,
        assignee_id: Option<MemberId>,
        /// Optional handoff note the actor attached when assigning/handing off
        /// — context for the assignee. Only carried by a deliberate `assign`; a
        /// pull-claim or unassign has none.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<String>,
        thread: Thread,
    },
    /// A thread whose last blocking dependency just reached a terminal state,
    /// so it is now ready to be claimed. Derived, not stored — a reactive push
    /// of the readiness that `dependencies_satisfied` computes on demand, so a
    /// waiting agent needn't poll.
    ThreadReady {
        occurred_at: DateTime<Utc>,
        workspace_id: WorkspaceId,
        channel_id: ChannelId,
        thread_id: ThreadId,
        thread: Thread,
    },
    /// A task produced (or revised) its structured result. A small "go fetch"
    /// pointer — a waiter reacts and reads the result via `get_thread_result`,
    /// so the payload isn't carried inline. Derived + local: not federatable.
    ThreadResultSet {
        occurred_at: DateTime<Utc>,
        workspace_id: WorkspaceId,
        channel_id: ChannelId,
        thread_id: ThreadId,
        produced_by: MemberId,
    },
    /// A durable human-approval gate was opened. The optional thread/channel
    /// context is absent for workspace-level questions. Locally derived: a
    /// federated peer cannot open human-control gates on this deployment.
    ApprovalRequested {
        occurred_at: DateTime<Utc>,
        workspace_id: WorkspaceId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        channel_id: Option<ChannelId>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        thread_id: Option<ThreadId>,
        gate_id: ApprovalGateId,
        requested_by: MemberId,
    },
    /// An explicit dispatch block was set. Carries the reason and the
    /// blocker's note. Locally derived: not federatable.
    ThreadBlocked {
        occurred_at: DateTime<Utc>,
        workspace_id: WorkspaceId,
        channel_id: ChannelId,
        thread_id: ThreadId,
        reason: BlockedReason,
        set_by: MemberId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<String>,
    },
    /// An explicit dispatch block was cleared. A waiter observing this can
    /// claim the thread (subject to DAG readiness and the other `claim_next`
    /// clauses). Carries the reason that resolved, not a payload —
    /// `get_thread_block` is now `None`. Locally derived: not federatable.
    BlockedResolved {
        occurred_at: DateTime<Utc>,
        workspace_id: WorkspaceId,
        channel_id: ChannelId,
        thread_id: ThreadId,
        reason: BlockedReason,
        resolved_by: MemberId,
    },
    /// An agent declared its status on a thread. Carries the status and the
    /// one-sentence note. Locally derived: not federatable.
    StatusDeclared {
        occurred_at: DateTime<Utc>,
        workspace_id: WorkspaceId,
        channel_id: ChannelId,
        thread_id: ThreadId,
        status: DeclaredStatus,
        note: String,
        declared_by: MemberId,
    },
    /// A claim's lease lapsed and the thread went back to the queue. Emitted by
    /// the claim reaper within a tick of the deadline, or by `claim_next` when
    /// it takes over a lease that lapsed between ticks, once either way —
    /// `member_id` is the *previous* holder whose claim expired, so a
    /// supervisor can react to a dead/stalled agent without polling. A
    /// locally-derived signal (this deployment's clock): not federatable. A
    /// lease on a thread in review is not reaped and emits nothing.
    ClaimExpired {
        occurred_at: DateTime<Utc>,
        workspace_id: WorkspaceId,
        channel_id: ChannelId,
        thread_id: ThreadId,
        /// The previous holder whose lease expired.
        member_id: MemberId,
        thread: Thread,
    },
    /// A leased claim went unacknowledged: its holder took the thread but has
    /// not called `acknowledge_claim` within the server's acknowledgement
    /// window (`MAIDAN_CLAIM_ACK_TIMEOUT_SECS`). The agent may have crashed
    /// right after claiming, or never started. Emitted once per claim by the
    /// claim reaper while the lease is still live; the claim itself is left
    /// alone (the lease decides when it comes back). A locally-derived signal
    /// (this deployment's clock): not federatable.
    ClaimUnacknowledged {
        occurred_at: DateTime<Utc>,
        workspace_id: WorkspaceId,
        channel_id: ChannelId,
        thread_id: ThreadId,
        /// The holder that has not acknowledged.
        member_id: MemberId,
        /// When the claim was taken.
        claimed_at: DateTime<Utc>,
        thread: Thread,
    },
    /// A claimed run was stopped because it exceeded its budget envelope — a
    /// hard stop, distinct from a normal close (success). The claim is released
    /// and the run is dead-lettered. A locally-derived signal (this
    /// deployment's budget accounting): not federatable.
    ClaimFailed {
        occurred_at: DateTime<Utc>,
        workspace_id: WorkspaceId,
        channel_id: ChannelId,
        thread_id: ThreadId,
        /// The holder whose run was stopped.
        member_id: MemberId,
        /// Which budget dimension was exceeded (`BudgetReason::as_str`):
        /// `tokens` | `usd` | `turns` | `wall`.
        reason: String,
        thread: Thread,
    },
    /// One accepted, idempotent model-usage heartbeat. The PayerStamp binds
    /// price evidence and token tiers to the authenticated active claim.
    UsageReported {
        occurred_at: DateTime<Utc>,
        workspace_id: WorkspaceId,
        channel_id: ChannelId,
        thread_id: ThreadId,
        usage_report_id: uuid::Uuid,
        stamp: PayerStamp,
        turns: i64,
        budget: ThreadBudget,
    },
    /// The GitHub PR linked to a thread was **merged** — the work landed. A
    /// derived fact projected from an inbound `pull_request` (`action=closed`,
    /// `merged=true`) webhook on a linked issue/PR; the room "steals the landed
    /// fact" without becoming a CI/automation product — the thread's FSM is not
    /// auto-transitioned. A locally-derived projector signal: not federatable.
    ThreadLanded {
        occurred_at: DateTime<Utc>,
        workspace_id: WorkspaceId,
        channel_id: ChannelId,
        thread_id: ThreadId,
        /// The GitHub repo (`owner/name`) whose merged PR landed this work.
        repo: String,
        /// The merged PR number (in the shared issue/PR number namespace).
        pr_number: i64,
        /// The GitHub login that merged it, when the webhook carried it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        merged_by: Option<String>,
        /// The merge commit sha, when present.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        merge_commit_sha: Option<String>,
        /// The PR title, when present.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        title: Option<String>,
    },
    /// A reviewer gave a verdict on a thread. `submit_review` appends one for
    /// every verdict, approve or request-changes, in the verdict's own
    /// transaction, so a waiter reacts to a review instead of polling for it.
    /// The note is not carried: it stays with the review history, which the
    /// worker reads in the thread context. Local governance: not federatable.
    ReviewSubmitted {
        occurred_at: DateTime<Utc>,
        workspace_id: WorkspaceId,
        channel_id: ChannelId,
        thread_id: ThreadId,
        /// The member the review is recorded under.
        reviewer_id: MemberId,
        /// The delegate that submitted it for `reviewer_id`, when one did.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        actor_id: Option<MemberId>,
        decision: crate::ReviewDecision,
        /// Whether this verdict sent the thread back to `open` for rework. A
        /// change request that sends nothing back is still recorded.
        sent_back: bool,
        /// The member who last took hold of the thread when the verdict was
        /// given: whose work was reviewed. `None` when nobody has held it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        worker_id: Option<MemberId>,
    },
    /// A thread's wait timer lapsed. Fired by *this* deployment's wait sweeper
    /// when a wait passed its deadline unsatisfied. The escalation `policy`
    /// names what the sweeper did (`notify` reaches the owner; `park`
    /// additionally marks the thread unclaimable) — **never a decision**. A
    /// locally-derived timer signal: not federatable.
    WaitTimedOut {
        occurred_at: DateTime<Utc>,
        workspace_id: WorkspaceId,
        channel_id: ChannelId,
        thread_id: ThreadId,
        /// The escalation applied (`EscalationPolicy::as_str`): `notify` | `park`.
        policy: String,
        /// Why the thread was waiting, if a reason was recorded.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    /// A spawn was refused by the workspace's spawn budget — a child thread
    /// past `max_children`/`max_depth`, or a tool call past `max_tools`.
    /// **Observability only**: the refusal itself is the `SpawnRejected` error
    /// the caller already received (REST 409 / MCP InvalidParams); this event
    /// is how an operator sees *which* members keep pushing a claim past its
    /// fan-out cap, instead of having to read logs. A locally-derived
    /// governance signal: not federatable.
    ThreadSpawnDenied {
        occurred_at: DateTime<Utc>,
        workspace_id: WorkspaceId,
        channel_id: ChannelId,
        /// The parent thread whose fan-out was capped (`children`/`depth`), or the
        /// thread whose tool calls were capped (`tools`).
        thread_id: ThreadId,
        /// Who tried to spawn — `None` only for an unattributed caller (a
        /// bypass-auth deployment), like the audit trail's `actor_id`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        member_id: Option<MemberId>,
        /// Which axis refused it (`SpawnAxis::as_str`): `children` | `depth` | `tools`.
        axis: String,
        /// The configured cap on that axis.
        limit: i64,
        /// What the thread already holds on that axis.
        observed: i64,
    },
    /// A projector egress link is broken and has been disabled: a delivery
    /// failed with an auth/config-class error (GitHub 401/403/404, Slack
    /// `invalid_auth`/`channel_not_found`/…) — a wrong token, a revoked scope,
    /// a deleted channel. Retrying cannot fix any of those, so the link is
    /// turned off, the delivery dead-letters, and this says so loudly instead
    /// of the queue grinding through eight attempts per message forever.
    ///
    /// Re-linking the channel/issue clears the disabled flag. A locally-derived
    /// operations signal about this deployment's own credentials: not
    /// federatable.
    ProjectorMisconfigured {
        occurred_at: DateTime<Utc>,
        workspace_id: WorkspaceId,
        /// The link's channel, resolved best-effort from the thread (`None` if it
        /// could not be read — the event is never withheld for want of context).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        channel_id: Option<ChannelId>,
        thread_id: ThreadId,
        /// The external surface (`EgressSurface::as_str`): `slack` | `github`.
        surface: String,
        /// The per-surface destination: a Slack channel id, or `owner/name#123`.
        selector: String,
        /// What the surface said, verbatim — the operator's actual diagnostic.
        error: String,
    },
    /// A recipe-backed schedule was due but its previous run is still in
    /// flight, so the sweeper skipped this firing — no new run.
    ScheduleSkipped {
        occurred_at: DateTime<Utc>,
        workspace_id: WorkspaceId,
        channel_id: ChannelId,
        schedule_id: TaskScheduleId,
        recipe_id: RecipeId,
        /// Why the firing was skipped (e.g. the prior run is not yet terminal).
        reason: String,
    },
    /// A member was frozen (the kill-switch): their active claims were
    /// released and `claim_next` now refuses them. Appended in the freeze's own
    /// transaction; a re-freeze appends another. Workspace-scoped, no channel,
    /// so every subscriber of the workspace sees it. Locally derived
    /// governance: not federatable.
    MemberFrozen {
        occurred_at: DateTime<Utc>,
        workspace_id: WorkspaceId,
        /// The member who was frozen.
        member_id: MemberId,
        frozen_by: MemberId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
        /// How many claimed threads the freeze returned to the queue.
        released: i64,
    },
    /// A member's freeze was lifted, so `claim_next` serves them again. Only
    /// an unfreeze that removed a freeze appends one.
    MemberUnfrozen {
        occurred_at: DateTime<Utc>,
        workspace_id: WorkspaceId,
        /// The member whose freeze was lifted.
        member_id: MemberId,
        unfrozen_by: MemberId,
    },
    MessagePosted {
        occurred_at: DateTime<Utc>,
        workspace_id: WorkspaceId,
        channel_id: ChannelId,
        thread_id: ThreadId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        dm_conversation_id: Option<DmConversationId>,
        message: Message,
        /// The message's words as the log stores them. `None` in memory;
        /// `Some` on a payload read back whose key is gone, where `message`
        /// has an empty body ([`crate::content_seal`]).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sealed: Option<crate::SealedContent>,
    },
    MessageEdited {
        occurred_at: DateTime<Utc>,
        workspace_id: WorkspaceId,
        channel_id: ChannelId,
        thread_id: ThreadId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        dm_conversation_id: Option<DmConversationId>,
        editor_id: MemberId,
        message: Message,
        /// See [`Event::MessagePosted`].
        #[serde(default, skip_serializing_if = "Option::is_none")]
        sealed: Option<crate::SealedContent>,
    },
    MessageTombstoned {
        occurred_at: DateTime<Utc>,
        workspace_id: WorkspaceId,
        channel_id: ChannelId,
        thread_id: ThreadId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        dm_conversation_id: Option<DmConversationId>,
        message_id: MessageId,
    },
    MentionRecorded {
        occurred_at: DateTime<Utc>,
        workspace_id: WorkspaceId,
        thread_id: ThreadId,
        message_id: MessageId,
        member_id: MemberId,
    },
    VoteCast {
        occurred_at: DateTime<Utc>,
        workspace_id: WorkspaceId,
        thread_id: ThreadId,
        message_id: MessageId,
        member_id: MemberId,
        vote_kind: String,
    },
    ReactionAdded {
        occurred_at: DateTime<Utc>,
        workspace_id: WorkspaceId,
        thread_id: ThreadId,
        message_id: MessageId,
        member_id: MemberId,
        emoji: String,
    },
    ReactionRemoved {
        occurred_at: DateTime<Utc>,
        workspace_id: WorkspaceId,
        thread_id: ThreadId,
        message_id: MessageId,
        member_id: MemberId,
        emoji: String,
    },
    MessagePinned {
        occurred_at: DateTime<Utc>,
        workspace_id: WorkspaceId,
        channel_id: ChannelId,
        thread_id: ThreadId,
        message_id: MessageId,
        member_id: MemberId,
    },
    MessageUnpinned {
        occurred_at: DateTime<Utc>,
        workspace_id: WorkspaceId,
        channel_id: ChannelId,
        thread_id: ThreadId,
        message_id: MessageId,
        member_id: MemberId,
    },
    ReferenceAdded {
        occurred_at: DateTime<Utc>,
        reference: Reference,
    },
    ArtifactUpserted {
        occurred_at: DateTime<Utc>,
        artifact: Artifact,
    },
    /// A memory block's value was rewritten. A small "go fetch" pointer — a
    /// waiter reacts and reads the value via `get_memory_block`, so the
    /// (possibly large) value isn't carried inline. Derived + local: not
    /// federatable. `updated_by` is the member who rewrote it.
    MemoryBlockUpdated {
        occurred_at: DateTime<Utc>,
        workspace_id: WorkspaceId,
        block_id: MemoryBlockId,
        label: String,
        updated_by: MemberId,
    },
}

impl Event {
    pub fn kind(&self) -> EventKind {
        match self {
            Self::WorkspaceCreated { .. } => EventKind::WorkspaceCreated,
            Self::MemberJoined { .. } => EventKind::MemberJoined,
            Self::ChannelCreated { .. } => EventKind::ChannelCreated,
            Self::ThreadCreated { .. } => EventKind::ThreadCreated,
            Self::ThreadStateChanged { .. } => EventKind::ThreadStateChanged,
            Self::ThreadAssignmentChanged { .. } => EventKind::ThreadAssignmentChanged,
            Self::ThreadReady { .. } => EventKind::ThreadReady,
            Self::ThreadResultSet { .. } => EventKind::ThreadResultSet,
            Self::ApprovalRequested { .. } => EventKind::ApprovalRequested,
            Self::ThreadBlocked { .. } => EventKind::ThreadBlocked,
            Self::BlockedResolved { .. } => EventKind::BlockedResolved,
            Self::StatusDeclared { .. } => EventKind::StatusDeclared,
            Self::ClaimExpired { .. } => EventKind::ClaimExpired,
            Self::ClaimUnacknowledged { .. } => EventKind::ClaimUnacknowledged,
            Self::ClaimFailed { .. } => EventKind::ClaimFailed,
            Self::UsageReported { .. } => EventKind::UsageReported,
            Self::ThreadLanded { .. } => EventKind::ThreadLanded,
            Self::ReviewSubmitted { .. } => EventKind::ReviewSubmitted,
            Self::WaitTimedOut { .. } => EventKind::WaitTimedOut,
            Self::ScheduleSkipped { .. } => EventKind::ScheduleSkipped,
            Self::ThreadSpawnDenied { .. } => EventKind::ThreadSpawnDenied,
            Self::ProjectorMisconfigured { .. } => EventKind::ProjectorMisconfigured,
            Self::MemberFrozen { .. } => EventKind::MemberFrozen,
            Self::MemberUnfrozen { .. } => EventKind::MemberUnfrozen,
            Self::MessagePosted { .. } => EventKind::MessagePosted,
            Self::MessageEdited { .. } => EventKind::MessageEdited,
            Self::MessageTombstoned { .. } => EventKind::MessageTombstoned,
            Self::MentionRecorded { .. } => EventKind::MentionRecorded,
            Self::VoteCast { .. } => EventKind::VoteCast,
            Self::ReactionAdded { .. } => EventKind::ReactionAdded,
            Self::ReactionRemoved { .. } => EventKind::ReactionRemoved,
            Self::MessagePinned { .. } => EventKind::MessagePinned,
            Self::MessageUnpinned { .. } => EventKind::MessageUnpinned,
            Self::MemoryBlockUpdated { .. } => EventKind::MemoryBlockUpdated,
            Self::ReferenceAdded { .. } => EventKind::ReferenceAdded,
            Self::ArtifactUpserted { .. } => EventKind::ArtifactUpserted,
        }
    }

    pub fn occurred_at(&self) -> DateTime<Utc> {
        match self {
            Self::WorkspaceCreated { occurred_at, .. }
            | Self::MemberJoined { occurred_at, .. }
            | Self::ChannelCreated { occurred_at, .. }
            | Self::ThreadCreated { occurred_at, .. }
            | Self::ThreadStateChanged { occurred_at, .. }
            | Self::ThreadAssignmentChanged { occurred_at, .. }
            | Self::ThreadReady { occurred_at, .. }
            | Self::ThreadResultSet { occurred_at, .. }
            | Self::ApprovalRequested { occurred_at, .. }
            | Self::ThreadBlocked { occurred_at, .. }
            | Self::BlockedResolved { occurred_at, .. }
            | Self::StatusDeclared { occurred_at, .. }
            | Self::ClaimExpired { occurred_at, .. }
            | Self::ClaimUnacknowledged { occurred_at, .. }
            | Self::ClaimFailed { occurred_at, .. }
            | Self::UsageReported { occurred_at, .. }
            | Self::ThreadLanded { occurred_at, .. }
            | Self::ReviewSubmitted { occurred_at, .. }
            | Self::WaitTimedOut { occurred_at, .. }
            | Self::ScheduleSkipped { occurred_at, .. }
            | Self::ThreadSpawnDenied { occurred_at, .. }
            | Self::ProjectorMisconfigured { occurred_at, .. }
            | Self::MemberFrozen { occurred_at, .. }
            | Self::MemberUnfrozen { occurred_at, .. }
            | Self::MessagePosted { occurred_at, .. }
            | Self::MessageEdited { occurred_at, .. }
            | Self::MessageTombstoned { occurred_at, .. }
            | Self::MentionRecorded { occurred_at, .. }
            | Self::VoteCast { occurred_at, .. }
            | Self::ReactionAdded { occurred_at, .. }
            | Self::ReactionRemoved { occurred_at, .. }
            | Self::MessagePinned { occurred_at, .. }
            | Self::MessageUnpinned { occurred_at, .. }
            | Self::ReferenceAdded { occurred_at, .. }
            | Self::ArtifactUpserted { occurred_at, .. }
            | Self::MemoryBlockUpdated { occurred_at, .. } => *occurred_at,
        }
    }

    pub fn workspace_id(&self) -> Option<WorkspaceId> {
        match self {
            Self::WorkspaceCreated { workspace, .. } => Some(workspace.id),
            Self::MemberJoined { workspace_id, .. }
            | Self::ChannelCreated { workspace_id, .. }
            | Self::ThreadCreated { workspace_id, .. }
            | Self::ThreadStateChanged { workspace_id, .. }
            | Self::ThreadAssignmentChanged { workspace_id, .. }
            | Self::ThreadReady { workspace_id, .. }
            | Self::ThreadResultSet { workspace_id, .. }
            | Self::ApprovalRequested { workspace_id, .. }
            | Self::ThreadBlocked { workspace_id, .. }
            | Self::BlockedResolved { workspace_id, .. }
            | Self::StatusDeclared { workspace_id, .. }
            | Self::ClaimExpired { workspace_id, .. }
            | Self::ClaimUnacknowledged { workspace_id, .. }
            | Self::ClaimFailed { workspace_id, .. }
            | Self::UsageReported { workspace_id, .. }
            | Self::ThreadLanded { workspace_id, .. }
            | Self::ReviewSubmitted { workspace_id, .. }
            | Self::WaitTimedOut { workspace_id, .. }
            | Self::ScheduleSkipped { workspace_id, .. }
            | Self::ThreadSpawnDenied { workspace_id, .. }
            | Self::ProjectorMisconfigured { workspace_id, .. }
            | Self::MemberFrozen { workspace_id, .. }
            | Self::MemberUnfrozen { workspace_id, .. }
            | Self::MessagePosted { workspace_id, .. }
            | Self::MessageEdited { workspace_id, .. }
            | Self::MessageTombstoned { workspace_id, .. }
            | Self::MentionRecorded { workspace_id, .. }
            | Self::VoteCast { workspace_id, .. }
            | Self::ReactionAdded { workspace_id, .. }
            | Self::ReactionRemoved { workspace_id, .. }
            | Self::MessagePinned { workspace_id, .. }
            | Self::MessageUnpinned { workspace_id, .. }
            | Self::MemoryBlockUpdated { workspace_id, .. } => Some(*workspace_id),
            Self::ReferenceAdded { .. } | Self::ArtifactUpserted { .. } => None,
        }
    }

    pub fn channel_id(&self) -> Option<ChannelId> {
        match self {
            Self::ChannelCreated { channel, .. } => Some(channel.id),
            Self::ThreadCreated { channel_id, .. }
            | Self::ThreadStateChanged { channel_id, .. }
            | Self::ThreadAssignmentChanged { channel_id, .. }
            | Self::ThreadReady { channel_id, .. }
            | Self::ThreadResultSet { channel_id, .. }
            | Self::BlockedResolved { channel_id, .. }
            | Self::ClaimExpired { channel_id, .. }
            | Self::ClaimUnacknowledged { channel_id, .. }
            | Self::ClaimFailed { channel_id, .. }
            | Self::UsageReported { channel_id, .. }
            | Self::ThreadLanded { channel_id, .. }
            | Self::ReviewSubmitted { channel_id, .. }
            | Self::WaitTimedOut { channel_id, .. }
            | Self::ScheduleSkipped { channel_id, .. }
            | Self::ThreadSpawnDenied { channel_id, .. }
            | Self::MessagePosted { channel_id, .. }
            | Self::MessageEdited { channel_id, .. }
            | Self::MessageTombstoned { channel_id, .. }
            | Self::MessagePinned { channel_id, .. }
            | Self::MessageUnpinned { channel_id, .. }
            | Self::StatusDeclared { channel_id, .. } => Some(*channel_id),
            // Already optional: resolved best-effort from the thread.
            Self::ProjectorMisconfigured { channel_id, .. } => *channel_id,
            Self::ApprovalRequested { channel_id, .. } => *channel_id,
            _ => None,
        }
    }

    pub fn thread_id(&self) -> Option<ThreadId> {
        match self {
            Self::ThreadCreated { thread, .. } => Some(thread.id),
            Self::ThreadStateChanged { thread_id, .. } => Some(*thread_id),
            Self::ThreadAssignmentChanged { thread_id, .. } => Some(*thread_id),
            Self::ThreadReady { thread_id, .. } => Some(*thread_id),
            Self::ThreadResultSet { thread_id, .. } => Some(*thread_id),
            Self::ApprovalRequested { thread_id, .. } => *thread_id,
            Self::BlockedResolved { thread_id, .. } => Some(*thread_id),
            Self::ClaimExpired { thread_id, .. } => Some(*thread_id),
            Self::ClaimUnacknowledged { thread_id, .. } => Some(*thread_id),
            Self::ClaimFailed { thread_id, .. } => Some(*thread_id),
            Self::UsageReported { thread_id, .. } => Some(*thread_id),
            Self::ThreadLanded { thread_id, .. } => Some(*thread_id),
            Self::ReviewSubmitted { thread_id, .. } => Some(*thread_id),
            Self::WaitTimedOut { thread_id, .. } => Some(*thread_id),
            Self::ThreadSpawnDenied { thread_id, .. } => Some(*thread_id),
            Self::ProjectorMisconfigured { thread_id, .. } => Some(*thread_id),
            Self::MessagePosted { thread_id, .. }
            | Self::MessageEdited { thread_id, .. }
            | Self::MessageTombstoned { thread_id, .. }
            | Self::MentionRecorded { thread_id, .. }
            | Self::VoteCast { thread_id, .. }
            | Self::ReactionAdded { thread_id, .. }
            | Self::ReactionRemoved { thread_id, .. }
            | Self::MessagePinned { thread_id, .. }
            | Self::MessageUnpinned { thread_id, .. }
            | Self::StatusDeclared { thread_id, .. } => Some(*thread_id),
            _ => None,
        }
    }

    pub fn dm_conversation_id(&self) -> Option<DmConversationId> {
        match self {
            Self::MessagePosted {
                dm_conversation_id, ..
            }
            | Self::MessageEdited {
                dm_conversation_id, ..
            }
            | Self::MessageTombstoned {
                dm_conversation_id, ..
            } => *dm_conversation_id,
            _ => None,
        }
    }

    pub fn member_id(&self) -> Option<MemberId> {
        match self {
            Self::MemberJoined { member, .. } => Some(member.id),
            Self::ThreadStateChanged { actor_id, .. } => Some(*actor_id),
            Self::ThreadAssignmentChanged { actor_id, .. } => Some(*actor_id),
            Self::ThreadResultSet { produced_by, .. } => Some(*produced_by),
            Self::ApprovalRequested { requested_by, .. } => Some(*requested_by),
            Self::BlockedResolved { resolved_by, .. } => Some(*resolved_by),
            Self::ClaimExpired { member_id, .. } => Some(*member_id),
            Self::ClaimUnacknowledged { member_id, .. } => Some(*member_id),
            Self::ClaimFailed { member_id, .. } => Some(*member_id),
            Self::MemberFrozen { member_id, .. } | Self::MemberUnfrozen { member_id, .. } => {
                Some(*member_id)
            }
            Self::ReviewSubmitted { reviewer_id, .. } => Some(*reviewer_id),
            Self::UsageReported { stamp, .. } => Some(stamp.reporter),
            Self::ThreadSpawnDenied { member_id, .. } => *member_id,
            Self::MentionRecorded { member_id, .. }
            | Self::VoteCast { member_id, .. }
            | Self::ReactionAdded { member_id, .. }
            | Self::ReactionRemoved { member_id, .. }
            | Self::MessagePinned { member_id, .. }
            | Self::MessageUnpinned { member_id, .. } => Some(*member_id),
            Self::MessageEdited { editor_id, .. } => Some(*editor_id),
            Self::MemoryBlockUpdated { updated_by, .. } => Some(*updated_by),
            Self::StatusDeclared { declared_by, .. } => Some(*declared_by),
            _ => None,
        }
    }
}

/// Event plus persistent log id from `maidan_events` (set on publish from the server).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BusEnvelope {
    pub log_id: i64,
    #[serde(flatten)]
    pub event: Event,
    /// Who acted and for whom, as the stored event records it. Parsing a
    /// stored payload into an [`Event`] drops it — `attribution` is not a field
    /// of any event — so it travels beside the event, and a live subscriber
    /// sees the same principal a replay does.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attribution: Option<crate::Attribution>,
    /// The server span the event was written under. Subscribers that call
    /// out continue this trace. Not a domain field and not part of the hash.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trace: Option<crate::TraceContext>,
}

impl BusEnvelope {
    /// For tests and direct bus use without a backing event log row.
    pub fn synthetic(event: Event) -> Self {
        Self {
            log_id: 0,
            event,
            attribution: None,
            trace: None,
        }
    }

    /// The envelope for a stored event, words opened and attribution
    /// included. Every path from the log to the bus goes through here.
    pub fn from_stored(stored: &StoredEvent) -> Result<Self, OpenEventError> {
        let payload = stored.opened_payload()?;
        let mut envelope = Self::from_payload(stored.id, payload)?;
        envelope.trace = stored.trace.clone();
        Ok(envelope)
    }

    /// [`Self::from_stored`] for a reader that holds the stored payload and its
    /// content key apart (the outbox relay's claim join).
    pub fn from_sealed_payload(
        log_id: i64,
        mut payload: serde_json::Value,
        content_key: Option<&crate::ContentKey>,
    ) -> Result<Self, OpenEventError> {
        crate::open_payload(&mut payload, content_key)?;
        Ok(Self::from_payload(log_id, payload)?)
    }

    fn from_payload(log_id: i64, payload: serde_json::Value) -> Result<Self, serde_json::Error> {
        // Lenient, like [`StoredEvent::attribution`]: a malformed value is a
        // chain-verification finding, not a reason to stall the live stream.
        let attribution = payload
            .get("attribution")
            .and_then(|value| serde_json::from_value(value.clone()).ok());
        Ok(Self {
            log_id,
            event: serde_json::from_value(payload)?,
            attribution,
            trace: None,
        })
    }
}

/// Why a stored event could not be opened into an [`Event`].
#[derive(Debug, thiserror::Error)]
pub enum OpenEventError {
    #[error("sealed words could not be opened: {0}")]
    Seal(#[from] crate::SealError),
    #[error("event payload does not parse: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EventFilter {
    pub workspace_id: Option<WorkspaceId>,
    pub channel_id: Option<ChannelId>,
    pub thread_id: Option<ThreadId>,
    pub dm_conversation_id: Option<DmConversationId>,
    pub member_id: Option<MemberId>,
    pub kinds: Option<HashSet<EventKind>>,
    /// Explicit channel allow-list; when set, only listed channels receive channel-scoped events.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub channel_grants: Option<Vec<ChannelId>>,
    /// Populated at subscribe time: private channels in the workspace not granted.
    #[serde(skip, default)]
    pub private_channel_deny: HashSet<ChannelId>,
    /// Populated when `channel_grants` is non-empty.
    #[serde(skip, default)]
    pub channel_event_allow: Option<HashSet<ChannelId>>,
}

impl EventFilter {
    pub fn all() -> Self {
        Self::default()
    }

    pub fn workspace(workspace_id: WorkspaceId) -> Self {
        Self {
            workspace_id: Some(workspace_id),
            ..Default::default()
        }
    }

    pub fn channel(channel_id: ChannelId) -> Self {
        Self {
            channel_id: Some(channel_id),
            ..Default::default()
        }
    }

    pub fn thread(thread_id: ThreadId) -> Self {
        Self {
            thread_id: Some(thread_id),
            ..Default::default()
        }
    }

    pub fn dm_conversation(dm_conversation_id: DmConversationId) -> Self {
        Self {
            dm_conversation_id: Some(dm_conversation_id),
            ..Default::default()
        }
    }

    pub fn member(member_id: MemberId) -> Self {
        Self {
            member_id: Some(member_id),
            ..Default::default()
        }
    }

    pub fn with_kinds<I: IntoIterator<Item = EventKind>>(mut self, kinds: I) -> Self {
        self.kinds = Some(kinds.into_iter().collect());
        self
    }

    pub fn matches_envelope(&self, envelope: &BusEnvelope) -> bool {
        self.matches(&envelope.event)
    }

    fn channel_is_granted(&self, channel_id: ChannelId) -> bool {
        self.channel_grants
            .as_ref()
            .is_some_and(|grants| grants.contains(&channel_id))
    }

    pub fn matches(&self, event: &Event) -> bool {
        if let Some(ws) = self.workspace_id {
            if event.workspace_id() != Some(ws) {
                return false;
            }
        }
        if let Event::ChannelCreated { channel, .. } = event {
            if channel.private
                && self.workspace_id.is_some()
                && !self.channel_is_granted(channel.id)
            {
                return false;
            }
        }
        if let Some(ch) = self.channel_id {
            if event.channel_id() != Some(ch) {
                return false;
            }
        }
        if let Some(ch) = event.channel_id() {
            if self.private_channel_deny.contains(&ch) {
                return false;
            }
            if let Some(ref allow) = self.channel_event_allow {
                if !allow.contains(&ch) {
                    return false;
                }
            }
        }
        if let Some(th) = self.thread_id {
            if event.thread_id() != Some(th) {
                return false;
            }
        }
        if let Some(dm) = self.dm_conversation_id {
            if event.dm_conversation_id() != Some(dm) {
                return false;
            }
        }
        if let Some(m) = self.member_id {
            if event.member_id() != Some(m) {
                return false;
            }
        }
        if let Some(ref kinds) = self.kinds {
            if !kinds.contains(&event.kind()) {
                return false;
            }
        }
        true
    }
}

/// Reconstruct a thread's message set from its events.
/// `MessagePosted`/`MessageEdited` both carry the full `Message`, so folding
/// them yields each message's body **as it stood** at the last event in
/// `events`; `MessageTombstoned` removes it. First-posted order is preserved
/// and non-message events are ignored. Pass the thread's events with `id <=
/// as_of` ([`crate` consumers use `Store::list_thread_events_through`]) to get
/// the as-of message set — deterministic over the immutable log, no current-row
/// reads. A message whose words were shredded since appears with an empty body.
pub fn reconstruct_messages_through(events: &[StoredEvent]) -> Vec<Message> {
    let mut order: Vec<MessageId> = Vec::new();
    let mut by_id: std::collections::HashMap<MessageId, Message> = std::collections::HashMap::new();
    for stored in events {
        let Ok(payload) = stored.opened_payload() else {
            continue;
        };
        let Ok(ev) = serde_json::from_value::<Event>(payload) else {
            continue;
        };
        match ev {
            Event::MessagePosted { message, .. } | Event::MessageEdited { message, .. } => {
                if !by_id.contains_key(&message.id) {
                    order.push(message.id);
                }
                by_id.insert(message.id, message);
            }
            Event::MessageTombstoned { message_id, .. } => {
                if by_id.remove(&message_id).is_some() {
                    order.retain(|id| *id != message_id);
                }
            }
            _ => {}
        }
    }
    order
        .iter()
        .filter_map(|id| by_id.get(id).cloned())
        .collect()
}

/// The thread row as the last event in `events` that carries one recorded it
/// (creation, a state change, an assignment, a lapsed, unacknowledged or failed
/// claim). Pass the thread's events with `id <= as_of`. A lease renewal that
/// writes no event stays as it was last logged. `None` when no event carries
/// the row.
pub fn reconstruct_thread_through(events: &[StoredEvent]) -> Option<Thread> {
    let mut thread = None;
    for stored in events {
        let Ok(payload) = stored.opened_payload() else {
            continue;
        };
        let Ok(ev) = serde_json::from_value::<Event>(payload) else {
            continue;
        };
        match ev {
            Event::ThreadCreated { thread: row, .. }
            | Event::ThreadStateChanged { thread: row, .. }
            | Event::ThreadAssignmentChanged { thread: row, .. }
            | Event::ThreadReady { thread: row, .. }
            | Event::ClaimExpired { thread: row, .. }
            | Event::ClaimUnacknowledged { thread: row, .. }
            | Event::ClaimFailed { thread: row, .. } => thread = Some(row),
            _ => {}
        }
    }
    thread
}

#[cfg(test)]
mod filter_tests {
    use super::*;
    use chrono::Utc;
    use std::collections::HashSet;

    fn sample_workspace(id: WorkspaceId) -> Workspace {
        Workspace {
            id,
            name: "ws".into(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            tombstoned_at: None,
        }
    }

    #[test]
    fn all_filter_matches_workspace_event() {
        let ws_id = WorkspaceId(uuid::Uuid::new_v4());
        let event = Event::WorkspaceCreated {
            occurred_at: Utc::now(),
            workspace: sample_workspace(ws_id),
        };
        assert!(EventFilter::all().matches(&event));
        assert!(EventFilter::all().matches_envelope(&BusEnvelope {
            log_id: 1,
            event,
            attribution: None,
            trace: None,
        }));
    }

    #[test]
    fn workspace_filter_rejects_other_workspace() {
        let ws_id = WorkspaceId(uuid::Uuid::new_v4());
        let other = WorkspaceId(uuid::Uuid::new_v4());
        let event = Event::WorkspaceCreated {
            occurred_at: Utc::now(),
            workspace: sample_workspace(ws_id),
        };
        assert!(EventFilter::workspace(ws_id).matches(&event));
        assert!(!EventFilter::workspace(other).matches(&event));
    }

    #[test]
    fn kinds_filter_limits_event_kind() {
        let ws_id = WorkspaceId(uuid::Uuid::new_v4());
        let event = Event::WorkspaceCreated {
            occurred_at: Utc::now(),
            workspace: sample_workspace(ws_id),
        };
        let kinds: HashSet<EventKind> = [EventKind::MessagePosted].into_iter().collect();
        assert!(!EventFilter::all().with_kinds(kinds).matches(&event));
    }
}

#[cfg(test)]
mod kind_tests {
    use super::*;

    /// The wire form is one source of truth: every variant's `as_str` must
    /// round-trip back through `parse`. The store once kept its own per-backend
    /// `parse_kind` copies, and a variant (`thread_assignment_changed`) shipped
    /// with its store parser missing — the event
    /// inserted but failed read-back and the transaction silently rolled back.
    /// The store now parses through `EventKind::parse`, so this single test
    /// guards the only remaining parser.
    #[test]
    fn all_variants_round_trip() {
        for &kind in EventKind::ALL {
            // Compile-time tripwire: a new variant that isn't listed in an arm
            // here fails to build (no wildcard), which is the reminder to also
            // add it to `EventKind::ALL` above so it gets round-trip-checked.
            match kind {
                EventKind::WorkspaceCreated
                | EventKind::MemberJoined
                | EventKind::ChannelCreated
                | EventKind::ThreadCreated
                | EventKind::ThreadStateChanged
                | EventKind::ThreadAssignmentChanged
                | EventKind::ThreadReady
                | EventKind::ThreadResultSet
                | EventKind::ApprovalRequested
                | EventKind::ThreadBlocked
                | EventKind::BlockedResolved
                | EventKind::ClaimExpired
                | EventKind::ClaimUnacknowledged
                | EventKind::ClaimFailed
                | EventKind::UsageReported
                | EventKind::ThreadLanded
                | EventKind::ReviewSubmitted
                | EventKind::WaitTimedOut
                | EventKind::ScheduleSkipped
                | EventKind::ThreadSpawnDenied
                | EventKind::ProjectorMisconfigured
                | EventKind::MemberFrozen
                | EventKind::MemberUnfrozen
                | EventKind::MessagePosted
                | EventKind::MessageEdited
                | EventKind::MessageTombstoned
                | EventKind::MentionRecorded
                | EventKind::VoteCast
                | EventKind::ReactionAdded
                | EventKind::ReactionRemoved
                | EventKind::MessagePinned
                | EventKind::MessageUnpinned
                | EventKind::ReferenceAdded
                | EventKind::ArtifactUpserted
                | EventKind::MemoryBlockUpdated
                | EventKind::StatusDeclared => {}
            }
            assert_eq!(
                EventKind::parse(kind.as_str()),
                Some(kind),
                "as_str/parse round-trip broken for {kind:?}"
            );
        }
    }

    #[test]
    fn all_lists_each_variant_once() {
        let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for &kind in EventKind::ALL {
            assert!(
                seen.insert(kind.as_str()),
                "EventKind::ALL lists {kind:?} more than once"
            );
        }
    }

    #[test]
    fn unknown_kind_does_not_parse() {
        assert_eq!(EventKind::parse("not_a_real_kind"), None);
    }

    #[test]
    fn type_id_round_trips_only_version_one() {
        for &kind in EventKind::ALL {
            let id = kind.type_id();
            assert_eq!(EventKind::parse_type_id(&id), Some(kind));
            assert!(
                id.starts_with("maidan.event.") && id.ends_with("/1"),
                "{id}"
            );
        }
        assert_eq!(
            EventKind::parse_type_id("maidan.event.message_posted/2"),
            None,
            "breaking = new type, not silently accepted as /1"
        );
        assert_eq!(
            EventKind::parse_type_id("maidan.event.not_a_real_kind/1"),
            None
        );
        assert_eq!(EventKind::parse_type_id("message_posted"), None);
    }

    /// The federation ingest allowlist. Collaboration-content kinds are
    /// federatable; `ArtifactUpserted` is not (blob bytes aren't federated),
    /// and `ThreadReady` is not (a locally-derived signal).
    #[test]
    fn federatable_allowlist_excludes_only_artifacts() {
        let non_federatable = [
            EventKind::ArtifactUpserted,
            EventKind::ThreadReady,
            EventKind::ThreadResultSet,
            EventKind::ApprovalRequested,
            EventKind::ThreadBlocked,
            EventKind::BlockedResolved,
            EventKind::ClaimExpired,
            EventKind::ClaimUnacknowledged,
            EventKind::ClaimFailed,
            EventKind::ThreadLanded,
            EventKind::ReviewSubmitted,
            EventKind::WaitTimedOut,
            EventKind::ScheduleSkipped,
            EventKind::ThreadSpawnDenied,
            EventKind::ProjectorMisconfigured,
            EventKind::MemberFrozen,
            EventKind::MemberUnfrozen,
            EventKind::MemoryBlockUpdated,
            EventKind::UsageReported,
            EventKind::StatusDeclared,
        ];
        for &kind in EventKind::ALL {
            let expected = !non_federatable.contains(&kind);
            assert_eq!(
                kind.federatable(),
                expected,
                "unexpected federatable() for {kind:?}"
            );
        }
        // Representative content kinds are accepted; artifacts + readiness are not.
        assert!(EventKind::MessagePosted.federatable());
        assert!(EventKind::MemberJoined.federatable());
        assert!(!EventKind::ArtifactUpserted.federatable());
        assert!(!EventKind::ThreadReady.federatable());
    }
}

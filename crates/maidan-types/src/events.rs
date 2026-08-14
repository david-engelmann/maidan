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

/// Row in the persistent `maidan_events` log (Cluster D.6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct StoredEvent {
    pub id: i64,
    pub kind: EventKind,
    pub workspace_id: Option<WorkspaceId>,
    pub channel_id: Option<ChannelId>,
    pub thread_id: Option<ThreadId>,
    pub payload: serde_json::Value,
    pub occurred_at: DateTime<Utc>,
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
            _ => None,
        }
    }

    /// Every variant, for exhaustive iteration (round-trip guards, catalogs).
    /// Kept in sync with the enum by the compile-time tripwire in
    /// `all_variants_round_trip` — a new variant fails that test's exhaustive
    /// match until it is listed here. `as_str`/`parse` are the single source of
    /// truth for the wire form; the store layer parses through `parse` (Cluster
    /// 181) so there is no per-backend copy to drift.
    pub const ALL: &'static [EventKind] = &[
        Self::WorkspaceCreated,
        Self::MemberJoined,
        Self::ChannelCreated,
        Self::ThreadCreated,
        Self::ThreadStateChanged,
        Self::ThreadAssignmentChanged,
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
    ];

    /// Whether a federated peer may push this event kind on ingest (Cluster 215
    /// federation ingest trust policy). **Allowlist-by-default via an exhaustive
    /// match**: a new event kind fails to compile here until it is consciously
    /// classified, so a peer can never inject an unreviewed kind into the local
    /// event log.
    ///
    /// All collaboration-content kinds are federatable — federation replicates the
    /// content event stream. **`ArtifactUpserted` is the exception**: federation
    /// replicates *events*, not artifact *blobs*, so an ingested `ArtifactUpserted`
    /// would announce a `sha256` whose bytes never arrive — a dangling reference.
    /// We therefore do not accept artifact-existence claims from peers.
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
        /// (Cluster 195) — context for the assignee. Only carried by a deliberate
        /// `assign`; a pull-claim or unassign has none.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        note: Option<String>,
        thread: Thread,
    },
    MessagePosted {
        occurred_at: DateTime<Utc>,
        workspace_id: WorkspaceId,
        channel_id: ChannelId,
        thread_id: ThreadId,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        dm_conversation_id: Option<DmConversationId>,
        message: Message,
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
            Self::MessagePosted { .. } => EventKind::MessagePosted,
            Self::MessageEdited { .. } => EventKind::MessageEdited,
            Self::MessageTombstoned { .. } => EventKind::MessageTombstoned,
            Self::MentionRecorded { .. } => EventKind::MentionRecorded,
            Self::VoteCast { .. } => EventKind::VoteCast,
            Self::ReactionAdded { .. } => EventKind::ReactionAdded,
            Self::ReactionRemoved { .. } => EventKind::ReactionRemoved,
            Self::MessagePinned { .. } => EventKind::MessagePinned,
            Self::MessageUnpinned { .. } => EventKind::MessageUnpinned,
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
            | Self::ArtifactUpserted { occurred_at, .. } => *occurred_at,
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
            | Self::MessagePosted { workspace_id, .. }
            | Self::MessageEdited { workspace_id, .. }
            | Self::MessageTombstoned { workspace_id, .. }
            | Self::MentionRecorded { workspace_id, .. }
            | Self::VoteCast { workspace_id, .. }
            | Self::ReactionAdded { workspace_id, .. }
            | Self::ReactionRemoved { workspace_id, .. }
            | Self::MessagePinned { workspace_id, .. }
            | Self::MessageUnpinned { workspace_id, .. } => Some(*workspace_id),
            Self::ReferenceAdded { .. } | Self::ArtifactUpserted { .. } => None,
        }
    }

    pub fn channel_id(&self) -> Option<ChannelId> {
        match self {
            Self::ChannelCreated { channel, .. } => Some(channel.id),
            Self::ThreadCreated { channel_id, .. }
            | Self::ThreadStateChanged { channel_id, .. }
            | Self::ThreadAssignmentChanged { channel_id, .. }
            | Self::MessagePosted { channel_id, .. }
            | Self::MessageEdited { channel_id, .. }
            | Self::MessageTombstoned { channel_id, .. }
            | Self::MessagePinned { channel_id, .. }
            | Self::MessageUnpinned { channel_id, .. } => Some(*channel_id),
            _ => None,
        }
    }

    pub fn thread_id(&self) -> Option<ThreadId> {
        match self {
            Self::ThreadCreated { thread, .. } => Some(thread.id),
            Self::ThreadStateChanged { thread_id, .. } => Some(*thread_id),
            Self::ThreadAssignmentChanged { thread_id, .. } => Some(*thread_id),
            Self::MessagePosted { thread_id, .. }
            | Self::MessageEdited { thread_id, .. }
            | Self::MessageTombstoned { thread_id, .. }
            | Self::MentionRecorded { thread_id, .. }
            | Self::VoteCast { thread_id, .. }
            | Self::ReactionAdded { thread_id, .. }
            | Self::ReactionRemoved { thread_id, .. }
            | Self::MessagePinned { thread_id, .. }
            | Self::MessageUnpinned { thread_id, .. } => Some(*thread_id),
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
            Self::MentionRecorded { member_id, .. }
            | Self::VoteCast { member_id, .. }
            | Self::ReactionAdded { member_id, .. }
            | Self::ReactionRemoved { member_id, .. }
            | Self::MessagePinned { member_id, .. }
            | Self::MessageUnpinned { member_id, .. } => Some(*member_id),
            Self::MessageEdited { editor_id, .. } => Some(*editor_id),
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
}

impl BusEnvelope {
    /// For tests and direct bus use without a backing event log row.
    pub fn synthetic(event: Event) -> Self {
        Self { log_id: 0, event }
    }
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
        assert!(EventFilter::all().matches_envelope(&BusEnvelope { log_id: 1, event }));
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
    /// round-trip back through `parse`. Before Cluster 181 the store kept its
    /// own per-backend `parse_kind` copies, and Cluster 171 shipped a variant
    /// (`thread_assignment_changed`) whose store parser was missing — the event
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
                | EventKind::ArtifactUpserted => {}
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

    /// Cluster 215: the federation ingest allowlist. Collaboration-content kinds
    /// are federatable; `ArtifactUpserted` is not (blob bytes aren't federated).
    #[test]
    fn federatable_allowlist_excludes_only_artifacts() {
        for &kind in EventKind::ALL {
            let expected = kind != EventKind::ArtifactUpserted;
            assert_eq!(
                kind.federatable(),
                expected,
                "unexpected federatable() for {kind:?}"
            );
        }
        // Representative content kinds are accepted; artifacts are not.
        assert!(EventKind::MessagePosted.federatable());
        assert!(EventKind::MemberJoined.federatable());
        assert!(!EventKind::ArtifactUpserted.federatable());
    }
}

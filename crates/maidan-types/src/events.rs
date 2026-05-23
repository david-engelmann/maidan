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

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum EventKind {
    WorkspaceCreated,
    MemberJoined,
    ChannelCreated,
    ThreadCreated,
    ThreadStateChanged,
    MessagePosted,
    MessageTombstoned,
    MentionRecorded,
    VoteCast,
    ReferenceAdded,
    ArtifactUpserted,
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
    MessagePosted {
        occurred_at: DateTime<Utc>,
        workspace_id: WorkspaceId,
        channel_id: ChannelId,
        thread_id: ThreadId,
        message: Message,
    },
    MessageTombstoned {
        occurred_at: DateTime<Utc>,
        workspace_id: WorkspaceId,
        channel_id: ChannelId,
        thread_id: ThreadId,
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
            Self::MessagePosted { .. } => EventKind::MessagePosted,
            Self::MessageTombstoned { .. } => EventKind::MessageTombstoned,
            Self::MentionRecorded { .. } => EventKind::MentionRecorded,
            Self::VoteCast { .. } => EventKind::VoteCast,
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
            | Self::MessagePosted { occurred_at, .. }
            | Self::MessageTombstoned { occurred_at, .. }
            | Self::MentionRecorded { occurred_at, .. }
            | Self::VoteCast { occurred_at, .. }
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
            | Self::MessagePosted { workspace_id, .. }
            | Self::MessageTombstoned { workspace_id, .. }
            | Self::MentionRecorded { workspace_id, .. }
            | Self::VoteCast { workspace_id, .. } => Some(*workspace_id),
            Self::ReferenceAdded { .. } | Self::ArtifactUpserted { .. } => None,
        }
    }

    pub fn channel_id(&self) -> Option<ChannelId> {
        match self {
            Self::ChannelCreated { channel, .. } => Some(channel.id),
            Self::ThreadCreated { channel_id, .. }
            | Self::ThreadStateChanged { channel_id, .. }
            | Self::MessagePosted { channel_id, .. }
            | Self::MessageTombstoned { channel_id, .. } => Some(*channel_id),
            _ => None,
        }
    }

    pub fn thread_id(&self) -> Option<ThreadId> {
        match self {
            Self::ThreadCreated { thread, .. } => Some(thread.id),
            Self::ThreadStateChanged { thread_id, .. } => Some(*thread_id),
            Self::MessagePosted { thread_id, .. }
            | Self::MessageTombstoned { thread_id, .. }
            | Self::MentionRecorded { thread_id, .. }
            | Self::VoteCast { thread_id, .. } => Some(*thread_id),
            _ => None,
        }
    }

    pub fn member_id(&self) -> Option<MemberId> {
        match self {
            Self::MemberJoined { member, .. } => Some(member.id),
            Self::ThreadStateChanged { actor_id, .. } => Some(*actor_id),
            Self::MentionRecorded { member_id, .. } | Self::VoteCast { member_id, .. } => {
                Some(*member_id)
            }
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EventFilter {
    pub workspace_id: Option<WorkspaceId>,
    pub channel_id: Option<ChannelId>,
    pub thread_id: Option<ThreadId>,
    pub member_id: Option<MemberId>,
    pub kinds: Option<HashSet<EventKind>>,
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

    pub fn matches(&self, event: &Event) -> bool {
        if let Some(ws) = self.workspace_id {
            if event.workspace_id() != Some(ws) {
                return false;
            }
        }
        if let Some(ch) = self.channel_id {
            if event.channel_id() != Some(ch) {
                return false;
            }
        }
        if let Some(th) = self.thread_id {
            if event.thread_id() != Some(th) {
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

//! Domain models. Each `<X>` has a paired `New<X>` for inserts so the
//! caller can build state-less values without populating server-assigned
//! fields (id, timestamps).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::ids::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
#[serde(rename_all = "snake_case")]
pub enum ThreadState {
    Open,
    Closed,
    Archived,
}

impl ThreadState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Closed => "closed",
            Self::Archived => "archived",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Thread {
    pub id: ThreadId,
    pub channel_id: ChannelId,
    pub title: Option<String>,
    pub state: ThreadState,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub tombstoned_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct NewThread {
    pub channel_id: ChannelId,
    pub title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: MessageId,
    pub thread_id: ThreadId,
    pub author_id: MemberId,
    pub body: String,
    pub metadata: serde_json::Value,
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Mention {
    pub message_id: MessageId,
    pub member_id: MemberId,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Vote {
    pub message_id: MessageId,
    pub member_id: MemberId,
    pub kind: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewVote {
    pub message_id: MessageId,
    pub member_id: MemberId,
    pub kind: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Reference {
    pub id: uuid::Uuid,
    pub src_kind: RefSide,
    pub src_id: uuid::Uuid,
    pub dst_kind: RefSide,
    pub dst_id: uuid::Uuid,
    pub relation: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewReference {
    pub src_kind: RefSide,
    pub src_id: uuid::Uuid,
    pub dst_kind: RefSide,
    pub dst_id: uuid::Uuid,
    pub relation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Artifact {
    pub id: ArtifactId,
    pub sha256: String,
    pub size_bytes: i64,
    pub mime_type: Option<String>,
    pub kind: String,
    pub uploaded_by: Option<MemberId>,
    pub created_at: DateTime<Utc>,
    pub tombstoned_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct NewArtifact {
    pub sha256: String,
    pub size_bytes: i64,
    pub mime_type: Option<String>,
    pub kind: String,
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

#[derive(Debug, Clone)]
pub struct NewAuditEvent {
    pub actor_id: Option<MemberId>,
    pub action: String,
    pub target_kind: Option<String>,
    pub target_id: Option<uuid::Uuid>,
    pub metadata: serde_json::Value,
}

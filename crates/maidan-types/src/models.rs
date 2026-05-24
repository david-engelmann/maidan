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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
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
    pub parent_thread_id: Option<ThreadId>,
    pub title: Option<String>,
    pub state: ThreadState,
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
pub struct ApiToken {
    pub id: ApiTokenId,
    pub workspace_id: WorkspaceId,
    pub member_id: MemberId,
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
    pub token_hash: String,
    pub label: Option<String>,
    pub capabilities: Vec<String>,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Peer {
    pub id: PeerId,
    pub workspace_id: WorkspaceId,
    pub name: String,
    pub base_url: String,
    #[serde(skip_serializing)]
    pub token_hash: String,
    pub enabled: bool,
    pub last_synced_event_id: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewPeer {
    pub workspace_id: WorkspaceId,
    pub name: String,
    pub base_url: String,
    pub token_hash: String,
}

#[derive(Debug, Clone)]
pub struct NewAuditEvent {
    pub actor_id: Option<MemberId>,
    pub action: String,
    pub target_kind: Option<String>,
    pub target_id: Option<uuid::Uuid>,
    pub metadata: serde_json::Value,
}

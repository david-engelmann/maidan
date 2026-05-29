//! Request DTOs. Response shapes use the `maidan_types` models directly
//! since they already derive `Serialize`. The DTOs here mirror the
//! domain `New<X>` structs but exist so the HTTP boundary can evolve
//! independently of the storage layer (e.g., omitting `workspace_id` in
//! a nested route in favor of the path parameter).

use chrono::{DateTime, Utc};
use maidan_types::{ApiTokenId, ArtifactKind, MemberId, MemberKind, RefSide, WorkspaceId};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateWorkspace {
    pub name: String,
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
pub struct CreateThread {
    pub title: Option<String>,
    pub parent_thread_id: Option<uuid::Uuid>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct TransitionThread {
    pub actor_id: uuid::Uuid,
    pub action: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateMessage {
    pub author_id: uuid::Uuid,
    pub body: String,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct EditMessageRequest {
    pub editor_id: uuid::Uuid,
    pub body: String,
    #[serde(default)]
    pub metadata: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateMention {
    pub member_id: uuid::Uuid,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateVote {
    pub member_id: uuid::Uuid,
    pub kind: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateReaction {
    pub member_id: uuid::Uuid,
    pub emoji: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RemoveReaction {
    pub member_id: uuid::Uuid,
    pub emoji: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PinMessage {
    pub message_id: uuid::Uuid,
    pub member_id: uuid::Uuid,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateReference {
    pub src_kind: RefSide,
    pub src_id: uuid::Uuid,
    pub dst_kind: RefSide,
    pub dst_id: uuid::Uuid,
    pub relation: String,
}

#[derive(Debug, Deserialize, ToSchema, IntoParams)]
pub struct ThreadContextQuery {
    #[serde(default = "default_limit")]
    pub message_limit: i64,
    #[serde(default = "default_transition_limit")]
    pub transition_limit: i64,
}

fn default_transition_limit() -> i64 {
    50
}

#[derive(Debug, Deserialize, ToSchema, IntoParams)]
pub struct ListMessagesQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
}

#[derive(Debug, Deserialize, ToSchema, IntoParams)]
pub struct ListMessageEditsQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
}

fn default_limit() -> i64 {
    100
}

#[derive(Debug, Deserialize, ToSchema, IntoParams)]
pub struct ListReferencesQuery {
    pub src_kind: RefSide,
    pub src_id: uuid::Uuid,
}

#[derive(Debug, Deserialize, ToSchema, IntoParams)]
pub struct ListEventsQuery {
    #[serde(default)]
    pub after_id: i64,
    #[serde(default = "default_limit")]
    pub limit: i64,
}

#[derive(Debug, Deserialize, IntoParams)]
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
}

#[derive(Debug, Deserialize, ToSchema, IntoParams)]
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
}

fn default_search_limit() -> i64 {
    25
}

#[derive(Debug, Deserialize, ToSchema, IntoParams)]
pub struct ListMentionsQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
}

#[derive(Debug, Deserialize, ToSchema, IntoParams)]
pub struct ListInboxQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
}

#[derive(Debug, Deserialize, ToSchema, IntoParams)]
pub struct UploadArtifactQuery {
    pub kind: ArtifactKind,
    pub mime_type: Option<String>,
    pub uploaded_by: Option<uuid::Uuid>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MultipartUploadResponse {
    pub upload_id: String,
    pub object_key: String,
}

#[derive(Debug, Deserialize, ToSchema, IntoParams)]
pub struct MultipartUploadQuery {
    pub object_key: String,
}

#[derive(Debug, Deserialize, ToSchema, IntoParams)]
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
pub struct CompleteMultipartArtifact {
    pub object_key: String,
    pub parts: Vec<MultipartPartInput>,
    pub kind: ArtifactKind,
    pub mime_type: Option<String>,
    pub uploaded_by: Option<uuid::Uuid>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct MintApiToken {
    pub label: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    pub expires_at: Option<DateTime<Utc>>,
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
    pub id: maidan_types::PeerId,
    pub workspace_id: maidan_types::WorkspaceId,
    pub remote_workspace_id: maidan_types::WorkspaceId,
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
pub struct MintApiTokenResponse {
    pub id: ApiTokenId,
    pub secret: String,
    pub workspace_id: WorkspaceId,
    pub member_id: MemberId,
    pub capabilities: Vec<String>,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize, ToSchema, IntoParams)]
pub struct OidcLoginQuery {
    pub workspace_id: uuid::Uuid,
    pub return_to: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema, IntoParams)]
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

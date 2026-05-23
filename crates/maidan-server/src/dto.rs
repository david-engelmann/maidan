//! Request DTOs. Response shapes use the `maidan_types` models directly
//! since they already derive `Serialize`. The DTOs here mirror the
//! domain `New<X>` structs but exist so the HTTP boundary can evolve
//! independently of the storage layer (e.g., omitting `workspace_id` in
//! a nested route in favor of the path parameter).

use chrono::{DateTime, Utc};
use maidan_types::{ApiTokenId, ArtifactKind, MemberId, MemberKind, RefSide, WorkspaceId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct CreateWorkspace {
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateMember {
    pub handle: String,
    pub display_name: Option<String>,
    pub kind: MemberKind,
}

#[derive(Debug, Deserialize)]
pub struct CreateChannel {
    pub name: String,
    pub topic: Option<String>,
    #[serde(default)]
    pub private: bool,
}

#[derive(Debug, Deserialize)]
pub struct CreateThread {
    pub title: Option<String>,
    pub parent_thread_id: Option<uuid::Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct TransitionThread {
    pub actor_id: uuid::Uuid,
    pub action: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateMessage {
    pub author_id: uuid::Uuid,
    pub body: String,
    #[serde(default)]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Deserialize)]
pub struct CreateMention {
    pub member_id: uuid::Uuid,
}

#[derive(Debug, Deserialize)]
pub struct CreateVote {
    pub member_id: uuid::Uuid,
    pub kind: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateReference {
    pub src_kind: RefSide,
    pub src_id: uuid::Uuid,
    pub dst_kind: RefSide,
    pub dst_id: uuid::Uuid,
    pub relation: String,
}

#[derive(Debug, Deserialize)]
pub struct ListMessagesQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
}

fn default_limit() -> i64 {
    100
}

#[derive(Debug, Deserialize)]
pub struct ListReferencesQuery {
    pub src_kind: RefSide,
    pub src_id: uuid::Uuid,
}

#[derive(Debug, Deserialize)]
pub struct ListEventsQuery {
    #[serde(default)]
    pub after_id: i64,
    #[serde(default = "default_limit")]
    pub limit: i64,
}

#[derive(Debug, Deserialize)]
pub struct SearchQuery {
    pub q: String,
    #[serde(default = "default_search_limit")]
    pub limit: i64,
}

fn default_search_limit() -> i64 {
    25
}

#[derive(Debug, Deserialize)]
pub struct ListMentionsQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
}

#[derive(Debug, Deserialize)]
pub struct UploadArtifactQuery {
    pub kind: ArtifactKind,
    pub mime_type: Option<String>,
    pub uploaded_by: Option<uuid::Uuid>,
}

#[derive(Debug, Deserialize)]
pub struct MintApiToken {
    pub label: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
pub struct MintApiTokenResponse {
    pub id: ApiTokenId,
    pub secret: String,
    pub workspace_id: WorkspaceId,
    pub member_id: MemberId,
    pub capabilities: Vec<String>,
    pub expires_at: Option<DateTime<Utc>>,
}

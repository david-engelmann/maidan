//! Request DTOs. Response shapes use the `maidan_types` models directly
//! since they already derive `Serialize`. The DTOs here mirror the
//! domain `New<X>` structs but exist so the HTTP boundary can evolve
//! independently of the storage layer (e.g., omitting `workspace_id` in
//! a nested route in favor of the path parameter).

use maidan_types::{MemberKind, RefSide};
use serde::Deserialize;

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
pub struct ListMentionsQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
}

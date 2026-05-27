//! OpenAPI-only schemas not tied to a single handler module.

use chrono::{DateTime, Utc};
use maidan_types::{ChannelId, MemberId, MessageId, ThreadId, WorkspaceId};
use serde::Serialize;
use utoipa::ToSchema;

#[derive(Debug, Serialize, ToSchema)]
pub struct LivenessOk {
    pub status: String,
}

/// Search hit (`GET /workspaces/{wid}/search`). `rank` is higher-is-better within
/// one response but not comparable across `mode=lexical` vs `mode=semantic` or SQLite vs Postgres.
#[derive(Debug, Serialize, ToSchema)]
pub struct SearchHit {
    pub message_id: MessageId,
    pub thread_id: ThreadId,
    pub channel_id: ChannelId,
    pub workspace_id: WorkspaceId,
    pub author_id: MemberId,
    pub posted_at: DateTime<Utc>,
    pub body: String,
    pub snippet: String,
    pub rank: f64,
    /// Present for `mode=semantic` on Postgres (active embedding model).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding_model: Option<String>,
}

//! OpenAPI-only schemas not tied to a single handler module.

use chrono::{DateTime, Utc};
use maidan_types::{ChannelId, MemberId, MessageId, ThreadId, WorkspaceId};
use serde::Serialize;
use utoipa::ToSchema;

#[derive(Debug, Serialize, ToSchema)]
pub struct LivenessOk {
    pub status: String,
}

/// Search hit (`GET /workspaces/{wid}/search`). `rank` is backend-specific
/// (higher is better within one response). `score` is normalized to `[0, 1]`
/// and comparable across Postgres and SQLite within the same `mode`.
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
    /// Normalized relevance in `[0, 1]`; comparable across backends within one mode.
    pub score: f64,
    /// Present for `mode=semantic` hits (active embedding model).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding_model: Option<String>,
}

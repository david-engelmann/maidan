use chrono::{DateTime, Utc};
use maidan_types::{ChannelId, MemberId, MessageId, ThreadId, WorkspaceId};
use serde::{Deserialize, Serialize};

/// A single search result. `rank` is backend-specific: lexical Postgres
/// uses `ts_rank_cd`; SQLite uses FTS5 `bm25` scaled negative; semantic
/// uses `1.0 - cosine_distance`. Higher is always better within one response.
/// `score` is normalized to `[0, 1]` and comparable across Postgres and SQLite
/// within the same search mode.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
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
    /// Normalized relevance in `[0, 1]` (higher is better). Comparable across
    /// backends within lexical or semantic mode.
    pub score: f64,
    /// Set for semantic hits: embedding model that matched.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embedding_model: Option<String>,
}

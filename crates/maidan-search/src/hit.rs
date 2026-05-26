use chrono::{DateTime, Utc};
use maidan_types::{ChannelId, MemberId, MessageId, ThreadId, WorkspaceId};
use serde::{Deserialize, Serialize};

/// A single search result. `rank` is backend-specific: lexical Postgres
/// uses `ts_rank_cd`; SQLite uses FTS5 `bm25` scaled negative; semantic
/// Postgres uses `1.0 - cosine_distance`. Higher is always better.
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
}

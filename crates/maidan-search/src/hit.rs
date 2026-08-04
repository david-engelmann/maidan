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

/// Byte budget the semantic fallback truncates `body` to when synthesizing a
/// snippet. Lexical hits already carry a bounded FTS snippet; semantic hits
/// have an empty snippet and lean on `body`, so `snippet_only` mode gives them
/// a truncated prefix instead of dropping their only content.
pub const SNIPPET_FALLBACK_BYTES: usize = 240;

impl SearchHit {
    /// Drop the full `body` for token-lean transport. Lexical hits keep their
    /// FTS `snippet`; semantic hits (empty snippet) get a truncated `body`
    /// prefix as the snippet so they still carry locatable content. Callers
    /// that need the whole message fetch it by `message_id`.
    pub fn into_snippet_only(mut self) -> Self {
        if self.snippet.is_empty() {
            self.snippet = truncate_on_char_boundary(&self.body, SNIPPET_FALLBACK_BYTES);
        }
        self.body = String::new();
        self
    }
}

/// Truncate to at most `max_bytes`, never splitting a UTF-8 code point, and
/// append an ellipsis when anything was dropped.
fn truncate_on_char_boundary(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    let mut out = s[..end].to_string();
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use maidan_types::{ChannelId, MemberId, MessageId, ThreadId, WorkspaceId};

    fn hit(body: &str, snippet: &str) -> SearchHit {
        SearchHit {
            message_id: MessageId(uuid::Uuid::nil()),
            thread_id: ThreadId(uuid::Uuid::nil()),
            channel_id: ChannelId(uuid::Uuid::nil()),
            workspace_id: WorkspaceId(uuid::Uuid::nil()),
            author_id: MemberId(uuid::Uuid::nil()),
            posted_at: chrono::Utc::now(),
            body: body.into(),
            snippet: snippet.into(),
            rank: 1.0,
            score: 1.0,
            embedding_model: None,
        }
    }

    #[test]
    fn snippet_only_drops_body_and_keeps_lexical_snippet() {
        let lean = hit("the full message body", "…full <mark>message</mark>…").into_snippet_only();
        assert_eq!(lean.body, "");
        assert_eq!(lean.snippet, "…full <mark>message</mark>…");
    }

    #[test]
    fn snippet_only_synthesizes_a_snippet_for_semantic_hits() {
        // Semantic hits arrive with an empty snippet; the body is their only content.
        let body = "x".repeat(SNIPPET_FALLBACK_BYTES + 50);
        let lean = hit(&body, "").into_snippet_only();
        assert_eq!(lean.body, "");
        assert!(!lean.snippet.is_empty());
        assert!(lean.snippet.ends_with('…'));
        // Truncated to the byte budget (+ the ellipsis).
        assert!(lean.snippet.len() <= SNIPPET_FALLBACK_BYTES + '…'.len_utf8());
    }

    #[test]
    fn truncation_respects_utf8_boundaries() {
        // Multi-byte chars straddling the budget must not panic or split.
        let body = "é".repeat(SNIPPET_FALLBACK_BYTES);
        let out = truncate_on_char_boundary(&body, SNIPPET_FALLBACK_BYTES);
        assert!(out.ends_with('…'));
        assert!(out.is_char_boundary(out.len()));
    }
}

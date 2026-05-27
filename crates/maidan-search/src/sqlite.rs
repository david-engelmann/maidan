//! SQLite FTS5 lexical search.
//!
//! Joins `maidan_messages_fts` to `maidan_messages` via the
//! `maidan_messages_fts_map` mapping table so the FTS5 rowid resolves
//! back to the canonical UUID message id.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use maidan_types::*;
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::SearchError;
use crate::filters::SearchFilters;
use crate::hit::SearchHit;
use crate::traits::Search;

#[derive(Debug, Clone)]
pub struct SqliteSearch {
    pool: SqlitePool,
}

impl SqliteSearch {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl Search for SqliteSearch {
    async fn search_messages(
        &self,
        workspace_id: WorkspaceId,
        query: &str,
        limit: i64,
        filters: &SearchFilters,
    ) -> Result<Vec<SearchHit>, SearchError> {
        let trimmed = query.trim();
        if trimmed.is_empty() {
            return Err(SearchError::InvalidQuery("empty query".into()));
        }
        let fts_query = escape_fts5_query(trimmed);
        let author_id = filters.author_id.map(|id| id.0);
        let channel_id = filters.channel_id.map(|id| id.0);
        let author_kind = filters.author_kind.map(|k| k.as_str().to_string());

        let rows = sqlx::query(
            r#"
            SELECT
                m.id            AS message_id,
                m.thread_id     AS thread_id,
                t.channel_id    AS channel_id,
                c.workspace_id  AS workspace_id,
                m.author_id     AS author_id,
                m.posted_at     AS posted_at,
                m.body          AS body,
                snippet(maidan_messages_fts, 0, '<mark>', '</mark>', '...', 20) AS snippet,
                -bm25(maidan_messages_fts) AS rank
            FROM maidan_messages_fts
            JOIN maidan_messages_fts_map map ON map.rowid = maidan_messages_fts.rowid
            JOIN maidan_messages m ON m.id = map.message_id
            JOIN maidan_threads t ON t.id = m.thread_id
            JOIN maidan_channels c ON c.id = t.channel_id
            JOIN maidan_members mem ON mem.id = m.author_id
            WHERE c.workspace_id = ?
              AND m.tombstoned_at IS NULL
              AND maidan_messages_fts MATCH ?
              AND (? IS NULL OR m.author_id = ?)
              AND (? IS NULL OR t.channel_id = ?)
              AND (? IS NULL OR mem.kind = ?)
            ORDER BY rank DESC, m.posted_at DESC
            LIMIT ?
            "#,
        )
        .bind(workspace_id.0)
        .bind(&fts_query)
        .bind(author_id)
        .bind(author_id)
        .bind(channel_id)
        .bind(channel_id)
        .bind(author_kind.as_deref())
        .bind(author_kind.as_deref())
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.iter().map(row_to_hit).collect())
    }

    async fn upsert_embedding(
        &self,
        _message_id: maidan_types::MessageId,
        _model: &str,
        _embedding: &[f32],
    ) -> Result<(), SearchError> {
        Err(SearchError::Unsupported("embeddings on sqlite"))
    }

    async fn semantic_search(
        &self,
        _workspace_id: WorkspaceId,
        _embedding: &[f32],
        _limit: i64,
        _filters: &SearchFilters,
    ) -> Result<Vec<SearchHit>, SearchError> {
        Err(SearchError::Unsupported("semantic search on sqlite"))
    }
}

fn row_to_hit(row: &sqlx::sqlite::SqliteRow) -> SearchHit {
    SearchHit {
        message_id: MessageId(row.get::<Uuid, _>("message_id")),
        thread_id: ThreadId(row.get::<Uuid, _>("thread_id")),
        channel_id: ChannelId(row.get::<Uuid, _>("channel_id")),
        workspace_id: WorkspaceId(row.get::<Uuid, _>("workspace_id")),
        author_id: MemberId(row.get::<Uuid, _>("author_id")),
        posted_at: row.get::<DateTime<Utc>, _>("posted_at"),
        body: row.get("body"),
        snippet: row.get("snippet"),
        rank: row.get::<f64, _>("rank"),
    }
}

/// FTS5 treats certain characters as operators (`"`, `(`, `)`, `*`, `:`).
/// Plain-language queries from HTTP need to be quoted token-by-token so
/// the user's words become literal phrase matches instead of syntax
/// errors. This isn't a SQL injection concern (we always bind the
/// parameter), only an FTS5 grammar concern.
fn escape_fts5_query(input: &str) -> String {
    input
        .split_whitespace()
        .filter(|t| !t.is_empty())
        .map(|t| {
            // FTS5 phrase syntax: "..."; double-quotes inside need to be doubled.
            let escaped = t.replace('"', "\"\"");
            format!("\"{escaped}\"")
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_handles_normal_words() {
        assert_eq!(escape_fts5_query("hello world"), "\"hello\" \"world\"");
    }

    #[test]
    fn escape_quotes_special_chars() {
        assert_eq!(escape_fts5_query("a(b)c"), "\"a(b)c\"");
    }

    #[test]
    fn escape_doubles_inner_quotes() {
        assert_eq!(escape_fts5_query("a\"b"), "\"a\"\"b\"");
    }
}

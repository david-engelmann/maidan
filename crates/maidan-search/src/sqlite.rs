//! SQLite FTS5 lexical search and embedding-backed semantic search.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use maidan_types::*;
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::embedding_provider::EmbeddingProvider;
use crate::embedding_tables;
use crate::error::SearchError;
use crate::filters::SearchFilters;
use crate::hit::SearchHit;
use crate::reindex::reindex_sqlite;
use crate::score::{apply_semantic_scores, normalize_lexical_scores};
use crate::sqlite_vec;
use crate::traits::Search;

#[derive(Debug, Clone)]
pub struct SqliteSearch {
    pool: SqlitePool,
}

impl SqliteSearch {
    pub fn new(pool: SqlitePool) -> Self {
        sqlite_vec::ensure_auto_extension();
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

        // RBAC pre-filter (Cluster 200): drop hits in denied channels at the
        // query level. SQLite has no array binding — expand `NOT IN (?, …)`.
        let deny_clause = deny_channels_clause_sqlite(&filters.deny_channels);
        let sql = format!(
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
              {deny_clause}
            ORDER BY rank DESC, m.posted_at DESC
            LIMIT ?
            "#
        );
        let mut q = sqlx::query(&sql)
            .bind(workspace_id.0)
            .bind(&fts_query)
            .bind(author_id)
            .bind(author_id)
            .bind(channel_id)
            .bind(channel_id)
            .bind(author_kind.as_deref())
            .bind(author_kind.as_deref());
        for cid in &filters.deny_channels {
            q = q.bind(cid.0);
        }
        let rows = q.bind(limit).fetch_all(&self.pool).await?;

        let mut hits: Vec<SearchHit> = rows.iter().map(row_to_hit).collect();
        normalize_lexical_scores(&mut hits);
        Ok(hits)
    }

    async fn upsert_embedding(
        &self,
        message_id: MessageId,
        model: &str,
        embedding: &[f32],
    ) -> Result<(), SearchError> {
        let table =
            embedding_tables::ensure_model_sqlite(&self.pool, model, embedding.len()).await?;
        let bytes = embedding_bytes(embedding);
        let sql = format!(
            r#"
            INSERT INTO {table} (message_id, embedding, updated_at)
            VALUES (?, ?, CURRENT_TIMESTAMP)
            ON CONFLICT (message_id) DO UPDATE SET
                embedding = excluded.embedding,
                updated_at = CURRENT_TIMESTAMP
            "#
        );
        sqlx::query(&sql)
            .bind(message_id.0)
            .bind(bytes)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    async fn semantic_search(
        &self,
        workspace_id: WorkspaceId,
        embedding: &[f32],
        limit: i64,
        filters: &SearchFilters,
        model: &str,
    ) -> Result<Vec<SearchHit>, SearchError> {
        let Some((table, registered_dim)) =
            embedding_tables::resolve_table_sqlite(&self.pool, model).await?
        else {
            return Ok(vec![]);
        };
        if embedding.len() != registered_dim {
            return Err(SearchError::InvalidQuery(format!(
                "expected {registered_dim}-dim vector for model {model}, got {}",
                embedding.len()
            )));
        }
        let author_id = filters.author_id.map(|id| id.0);
        let channel_id = filters.channel_id.map(|id| id.0);
        let author_kind = filters.author_kind.map(|k| k.as_str().to_string());

        let deny_clause = deny_channels_clause_sqlite(&filters.deny_channels);
        if sqlite_vec::vec_available(&self.pool).await {
            let query_bytes = embedding_bytes(embedding);
            let sql = format!(
                r#"
                SELECT
                    m.id            AS message_id,
                    m.thread_id     AS thread_id,
                    t.channel_id    AS channel_id,
                    c.workspace_id  AS workspace_id,
                    m.author_id     AS author_id,
                    m.posted_at     AS posted_at,
                    m.body          AS body,
                    1.0 - vec_distance_cosine(e.embedding, ?) AS rank
                FROM {table} e
                JOIN maidan_messages m ON m.id = e.message_id
                JOIN maidan_threads t ON t.id = m.thread_id
                JOIN maidan_channels c ON c.id = t.channel_id
                JOIN maidan_members mem ON mem.id = m.author_id
                WHERE c.workspace_id = ?
                  AND m.tombstoned_at IS NULL
                  AND (? IS NULL OR m.author_id = ?)
                  AND (? IS NULL OR t.channel_id = ?)
                  AND (? IS NULL OR mem.kind = ?)
                  {deny_clause}
                ORDER BY vec_distance_cosine(e.embedding, ?)
                LIMIT ?
                "#
            );
            let mut q = sqlx::query(&sql)
                .bind(&query_bytes)
                .bind(workspace_id.0)
                .bind(author_id)
                .bind(author_id)
                .bind(channel_id)
                .bind(channel_id)
                .bind(author_kind.as_deref())
                .bind(author_kind.as_deref());
            for cid in &filters.deny_channels {
                q = q.bind(cid.0);
            }
            let rows = q
                .bind(&query_bytes)
                .bind(limit)
                .fetch_all(&self.pool)
                .await?;

            let mut hits: Vec<SearchHit> = rows
                .iter()
                .map(|row| semantic_row_to_hit(row, model))
                .collect();
            apply_semantic_scores(&mut hits);
            return Ok(hits);
        }

        let sql = format!(
            r#"
            SELECT
                m.id            AS message_id,
                m.thread_id     AS thread_id,
                t.channel_id    AS channel_id,
                c.workspace_id  AS workspace_id,
                m.author_id     AS author_id,
                m.posted_at     AS posted_at,
                m.body          AS body,
                e.embedding     AS embedding
            FROM {table} e
            JOIN maidan_messages m ON m.id = e.message_id
            JOIN maidan_threads t ON t.id = m.thread_id
            JOIN maidan_channels c ON c.id = t.channel_id
            JOIN maidan_members mem ON mem.id = m.author_id
            WHERE c.workspace_id = ?
              AND m.tombstoned_at IS NULL
              AND (? IS NULL OR m.author_id = ?)
              AND (? IS NULL OR t.channel_id = ?)
              AND (? IS NULL OR mem.kind = ?)
              {deny_clause}
            "#
        );
        let mut q = sqlx::query(&sql)
            .bind(workspace_id.0)
            .bind(author_id)
            .bind(author_id)
            .bind(channel_id)
            .bind(channel_id)
            .bind(author_kind.as_deref())
            .bind(author_kind.as_deref());
        for cid in &filters.deny_channels {
            q = q.bind(cid.0);
        }
        let rows = q.fetch_all(&self.pool).await?;

        let mut hits: Vec<SearchHit> = rows
            .iter()
            .filter_map(|row| {
                let stored = row.get::<Vec<u8>, _>("embedding");
                let stored_vec = parse_embedding_bytes(&stored, registered_dim).ok()?;
                let distance = cosine_distance(embedding, &stored_vec);
                Some(SearchHit {
                    message_id: MessageId(row.get::<Uuid, _>("message_id")),
                    thread_id: ThreadId(row.get::<Uuid, _>("thread_id")),
                    channel_id: ChannelId(row.get::<Uuid, _>("channel_id")),
                    workspace_id: WorkspaceId(row.get::<Uuid, _>("workspace_id")),
                    author_id: MemberId(row.get::<Uuid, _>("author_id")),
                    posted_at: row.get::<DateTime<Utc>, _>("posted_at"),
                    body: row.get("body"),
                    snippet: String::new(),
                    rank: 1.0 - distance,
                    score: 0.0,
                    embedding_model: Some(model.to_string()),
                })
            })
            .collect();
        hits.sort_by(|a, b| {
            b.rank
                .partial_cmp(&a.rank)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        hits.truncate(limit as usize);
        apply_semantic_scores(&mut hits);
        Ok(hits)
    }

    async fn reindex_embeddings(
        &self,
        provider: &dyn EmbeddingProvider,
        workspace_id: Option<WorkspaceId>,
    ) -> Result<crate::reindex::ReindexReport, SearchError> {
        reindex_sqlite(&self.pool, self, provider, workspace_id).await
    }

    async fn ensure_model(&self, provider: &dyn EmbeddingProvider) -> Result<(), SearchError> {
        embedding_tables::ensure_model_sqlite(
            &self.pool,
            provider.model_name(),
            provider.dimension(),
        )
        .await?;
        Ok(())
    }
}

fn parse_embedding_bytes(bytes: &[u8], dimension: usize) -> Result<Vec<f32>, SearchError> {
    if bytes.len() != dimension * 4 {
        return Err(SearchError::InvalidQuery(format!(
            "expected {} embedding bytes, got {}",
            dimension * 4,
            bytes.len()
        )));
    }
    Ok(bytes
        .chunks_exact(4)
        .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect())
}

fn cosine_distance(a: &[f32], b: &[f32]) -> f64 {
    let mut dot = 0.0f64;
    let mut norm_a = 0.0f64;
    let mut norm_b = 0.0f64;
    for (x, y) in a.iter().zip(b.iter()) {
        let x = f64::from(*x);
        let y = f64::from(*y);
        dot += x * y;
        norm_a += x * x;
        norm_b += y * y;
    }
    if norm_a == 0.0 || norm_b == 0.0 {
        return 1.0;
    }
    1.0 - (dot / (norm_a.sqrt() * norm_b.sqrt()))
}

fn embedding_bytes(embedding: &[f32]) -> Vec<u8> {
    let mut out = Vec::with_capacity(embedding.len() * 4);
    for value in embedding {
        out.extend_from_slice(&value.to_le_bytes());
    }
    out
}

/// `AND t.channel_id NOT IN (?, …)` for the RBAC deny pre-filter, or empty when
/// there is nothing to deny (Cluster 200). SQLite has no array binding, so the
/// caller binds each `ChannelId` in `deny` after the fixed facet binds.
fn deny_channels_clause_sqlite(deny: &[ChannelId]) -> String {
    if deny.is_empty() {
        String::new()
    } else {
        let placeholders = vec!["?"; deny.len()].join(", ");
        format!("AND t.channel_id NOT IN ({placeholders})")
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
        score: 0.0,
        embedding_model: None,
    }
}

fn semantic_row_to_hit(row: &sqlx::sqlite::SqliteRow, model: &str) -> SearchHit {
    SearchHit {
        message_id: MessageId(row.get::<Uuid, _>("message_id")),
        thread_id: ThreadId(row.get::<Uuid, _>("thread_id")),
        channel_id: ChannelId(row.get::<Uuid, _>("channel_id")),
        workspace_id: WorkspaceId(row.get::<Uuid, _>("workspace_id")),
        author_id: MemberId(row.get::<Uuid, _>("author_id")),
        posted_at: row.get::<DateTime<Utc>, _>("posted_at"),
        body: row.get("body"),
        snippet: String::new(),
        rank: row.get::<f64, _>("rank"),
        score: 0.0,
        embedding_model: Some(model.to_string()),
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

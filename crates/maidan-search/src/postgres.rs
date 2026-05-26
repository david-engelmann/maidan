//! Postgres tsvector + ts_headline lexical search, plus pgvector-backed
//! semantic search.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use maidan_types::*;
use pgvector::Vector;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::SearchError;
use crate::filters::SearchFilters;
use crate::hit::SearchHit;
use crate::traits::Search;

/// Dimension of every embedding vector. Must match the schema column
/// declared in `migrations/postgres/0003_embeddings.sql`. Future
/// migrations widen or partition by model.
pub const EMBEDDING_DIM: usize = 1024;

#[derive(Debug, Clone)]
pub struct PostgresSearch {
    pool: PgPool,
}

impl PostgresSearch {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl Search for PostgresSearch {
    async fn search_messages(
        &self,
        workspace_id: WorkspaceId,
        query: &str,
        limit: i64,
        filters: &SearchFilters,
    ) -> Result<Vec<SearchHit>, SearchError> {
        if query.trim().is_empty() {
            return Err(SearchError::InvalidQuery("empty query".into()));
        }

        let author_id = filters.author_id.map(|id| id.0);
        let channel_id = filters.channel_id.map(|id| id.0);
        let author_kind = filters.author_kind.map(|k| k.as_str().to_string());

        let rows = sqlx::query(
            r#"
            WITH q AS (SELECT plainto_tsquery('english', $2) AS query)
            SELECT
                m.id            AS message_id,
                m.thread_id     AS thread_id,
                t.channel_id    AS channel_id,
                c.workspace_id  AS workspace_id,
                m.author_id     AS author_id,
                m.posted_at     AS posted_at,
                m.body          AS body,
                ts_headline(
                    'english',
                    m.body,
                    (SELECT query FROM q),
                    'StartSel=<mark>, StopSel=</mark>, MaxFragments=2, MinWords=3, MaxWords=20'
                )               AS snippet,
                ts_rank_cd(m.search_vec, (SELECT query FROM q)) AS rank
            FROM maidan_messages m
            JOIN maidan_threads t ON t.id = m.thread_id
            JOIN maidan_channels c ON c.id = t.channel_id
            JOIN maidan_members mem ON mem.id = m.author_id
            WHERE c.workspace_id = $1
              AND m.tombstoned_at IS NULL
              AND m.search_vec @@ (SELECT query FROM q)
              AND ($4::uuid IS NULL OR m.author_id = $4)
              AND ($5::uuid IS NULL OR t.channel_id = $5)
              AND ($6::text IS NULL OR mem.kind = $6)
            ORDER BY rank DESC, m.posted_at DESC
            LIMIT $3
            "#,
        )
        .bind(workspace_id.0)
        .bind(query)
        .bind(limit)
        .bind(author_id)
        .bind(channel_id)
        .bind(author_kind)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.iter().map(row_to_lexical_hit).collect())
    }

    async fn upsert_embedding(
        &self,
        message_id: MessageId,
        model: &str,
        embedding: &[f32],
    ) -> Result<(), SearchError> {
        if embedding.len() != EMBEDDING_DIM {
            return Err(SearchError::InvalidQuery(format!(
                "expected {EMBEDDING_DIM}-dim vector, got {}",
                embedding.len()
            )));
        }
        let vector = Vector::from(embedding.to_vec());
        sqlx::query(
            r#"
            INSERT INTO maidan_message_embeddings (message_id, model, embedding, updated_at)
            VALUES ($1, $2, $3, NOW())
            ON CONFLICT (message_id) DO UPDATE
                SET model = EXCLUDED.model,
                    embedding = EXCLUDED.embedding,
                    updated_at = NOW()
            "#,
        )
        .bind(message_id.0)
        .bind(model)
        .bind(vector)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn semantic_search(
        &self,
        workspace_id: WorkspaceId,
        embedding: &[f32],
        limit: i64,
    ) -> Result<Vec<SearchHit>, SearchError> {
        if embedding.len() != EMBEDDING_DIM {
            return Err(SearchError::InvalidQuery(format!(
                "expected {EMBEDDING_DIM}-dim vector, got {}",
                embedding.len()
            )));
        }
        let vector = Vector::from(embedding.to_vec());

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
                ''              AS snippet,
                1.0 - (e.embedding <=> $2) AS rank
            FROM maidan_message_embeddings e
            JOIN maidan_messages m ON m.id = e.message_id
            JOIN maidan_threads t ON t.id = m.thread_id
            JOIN maidan_channels c ON c.id = t.channel_id
            WHERE c.workspace_id = $1
              AND m.tombstoned_at IS NULL
            ORDER BY e.embedding <=> $2
            LIMIT $3
            "#,
        )
        .bind(workspace_id.0)
        .bind(vector)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.iter().map(row_to_semantic_hit).collect())
    }
}

fn row_to_lexical_hit(row: &sqlx::postgres::PgRow) -> SearchHit {
    SearchHit {
        message_id: MessageId(row.get::<Uuid, _>("message_id")),
        thread_id: ThreadId(row.get::<Uuid, _>("thread_id")),
        channel_id: ChannelId(row.get::<Uuid, _>("channel_id")),
        workspace_id: WorkspaceId(row.get::<Uuid, _>("workspace_id")),
        author_id: MemberId(row.get::<Uuid, _>("author_id")),
        posted_at: row.get::<DateTime<Utc>, _>("posted_at"),
        body: row.get("body"),
        snippet: row.get("snippet"),
        rank: row.get::<f32, _>("rank") as f64,
    }
}

fn row_to_semantic_hit(row: &sqlx::postgres::PgRow) -> SearchHit {
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

//! Postgres tsvector + ts_headline lexical search, plus pgvector-backed
//! semantic search.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use maidan_types::*;
use pgvector::Vector;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::embedding_provider::EmbeddingProvider;
use crate::embedding_tables;
use crate::error::SearchError;
use crate::filters::SearchFilters;
use crate::hit::SearchHit;
use crate::query::use_websearch_to_tsquery;
use crate::reindex::reindex_postgres;
use crate::score::{apply_semantic_scores, normalize_lexical_scores};
use crate::traits::Search;

/// Default embedding dimension (`hash-v1`). Per-model tables may use other sizes.
pub use crate::embedding_tables::DEFAULT_EMBEDDING_DIM as EMBEDDING_DIM;

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
        let websearch = use_websearch_to_tsquery(query);

        let rows = sqlx::query(
            r#"
            WITH q AS (
                SELECT CASE WHEN $7
                    THEN websearch_to_tsquery('english', $2)
                    ELSE plainto_tsquery('english', $2)
                END AS query
            )
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
        .bind(websearch)
        .fetch_all(&self.pool)
        .await?;

        let mut hits: Vec<SearchHit> = rows.iter().map(row_to_lexical_hit).collect();
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
            embedding_tables::ensure_model_postgres(&self.pool, model, embedding.len()).await?;
        let vector = Vector::from(embedding.to_vec());
        let sql = format!(
            r#"
            INSERT INTO {table} (message_id, embedding, updated_at)
            VALUES ($1, $2, NOW())
            ON CONFLICT (message_id) DO UPDATE
                SET embedding = EXCLUDED.embedding,
                    updated_at = NOW()
            "#
        );
        sqlx::query(&sql)
            .bind(message_id.0)
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
        filters: &SearchFilters,
        model: &str,
    ) -> Result<Vec<SearchHit>, SearchError> {
        let Some((table, registered_dim)) =
            embedding_tables::resolve_table_postgres(&self.pool, model).await?
        else {
            return Ok(vec![]);
        };
        if embedding.len() != registered_dim {
            return Err(SearchError::InvalidQuery(format!(
                "expected {registered_dim}-dim vector for model {model}, got {}",
                embedding.len()
            )));
        }
        let vector = Vector::from(embedding.to_vec());
        let author_id = filters.author_id.map(|id| id.0);
        let channel_id = filters.channel_id.map(|id| id.0);
        let author_kind = filters.author_kind.map(|k| k.as_str().to_string());

        let rows = sqlx::query(&format!(
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
            FROM {table} e
            JOIN maidan_messages m ON m.id = e.message_id
            JOIN maidan_threads t ON t.id = m.thread_id
            JOIN maidan_channels c ON c.id = t.channel_id
            JOIN maidan_members mem ON mem.id = m.author_id
            WHERE c.workspace_id = $1
              AND m.tombstoned_at IS NULL
              AND ($4::uuid IS NULL OR m.author_id = $4)
              AND ($5::uuid IS NULL OR t.channel_id = $5)
              AND ($6::text IS NULL OR mem.kind = $6)
            ORDER BY e.embedding <=> $2
            LIMIT $3
            "#
        ))
        .bind(workspace_id.0)
        .bind(vector)
        .bind(limit)
        .bind(author_id)
        .bind(channel_id)
        .bind(author_kind)
        .fetch_all(&self.pool)
        .await?;

        let mut hits: Vec<SearchHit> = rows
            .iter()
            .map(|row| row_to_semantic_hit(row, model))
            .collect();
        apply_semantic_scores(&mut hits);
        Ok(hits)
    }

    async fn reindex_embeddings(
        &self,
        provider: &dyn EmbeddingProvider,
        workspace_id: Option<WorkspaceId>,
    ) -> Result<crate::reindex::ReindexReport, SearchError> {
        reindex_postgres(&self.pool, self, provider, workspace_id).await
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
        score: 0.0,
        embedding_model: None,
    }
}

fn row_to_semantic_hit(row: &sqlx::postgres::PgRow, model: &str) -> SearchHit {
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
        embedding_model: Some(model.to_string()),
    }
}

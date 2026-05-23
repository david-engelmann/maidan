//! Postgres tsvector + ts_headline search.

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use maidan_types::*;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::SearchError;
use crate::hit::SearchHit;
use crate::traits::Search;

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
    ) -> Result<Vec<SearchHit>, SearchError> {
        if query.trim().is_empty() {
            return Err(SearchError::InvalidQuery("empty query".into()));
        }

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
            WHERE c.workspace_id = $1
              AND m.tombstoned_at IS NULL
              AND m.search_vec @@ (SELECT query FROM q)
            ORDER BY rank DESC, m.posted_at DESC
            LIMIT $3
            "#,
        )
        .bind(workspace_id.0)
        .bind(query)
        .bind(limit)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.iter().map(row_to_hit).collect())
    }
}

fn row_to_hit(row: &sqlx::postgres::PgRow) -> SearchHit {
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

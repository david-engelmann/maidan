//! Postgres tsvector + ts_headline lexical search, plus pgvector-backed
//! semantic search.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use maidan_types::*;
use pgvector::Vector;
use sqlx::{PgPool, Row};
use uuid::Uuid;

/// How often the replica-replay-LSN cache is refreshed (Cluster 271). Matches the
/// store's poller cadence so search and store see the replica advance in lockstep.
const REPLICA_LSN_POLL_INTERVAL: Duration = Duration::from_millis(200);

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
    /// Read-replica pool for search reads (Cluster 271). Equals `pool` (and
    /// `has_replica` is false) unless a replica is configured, so single-primary
    /// deployments and tests are byte-unchanged.
    reader: PgPool,
    has_replica: bool,
    /// Cached replica `pg_last_wal_replay_lsn()`, refreshed by a background poller,
    /// so [`read_pool`](Self::read_pool) decides primary-vs-replica without a
    /// per-read query. Only meaningful when `has_replica`.
    replica_replay: Arc<AtomicU64>,
    hnsw: crate::hnsw::HnswParams,
    /// Cache of resolved `model → table_name` so a steady-state embedding upsert
    /// skips the `maidan_embedding_models` SELECT + `CREATE TABLE IF NOT EXISTS`
    /// checks on every call (Cluster 167, H6). A model's table never changes once
    /// registered.
    model_tables: std::sync::Arc<std::sync::Mutex<std::collections::HashMap<String, String>>>,
    /// How many search reads went to the replica vs the primary (Cluster 272), for
    /// `maidan_search_replica_reads_total`. Counted only when a replica is configured
    /// (a single-pool search leaves it at zero). The store's metrics-agnostic
    /// `ReadRoutingMetrics` pattern.
    read_routing: Arc<SearchReadMetrics>,
}

/// Cumulative search read-routing outcomes (Cluster 272). The server snapshots this
/// into `maidan_search_replica_reads_total{outcome}`; search stays metrics-agnostic
/// (no lag gauge here — the store's poller already emits `maidan_replica_lag_bytes`
/// for the same replica).
#[derive(Debug, Default)]
pub struct SearchReadMetrics {
    primary: AtomicU64,
    replica: AtomicU64,
}

impl SearchReadMetrics {
    /// `(primary, replica)` cumulative search read counts.
    pub fn snapshot(&self) -> (u64, u64) {
        (
            self.primary.load(Ordering::Relaxed),
            self.replica.load(Ordering::Relaxed),
        )
    }
}

impl PostgresSearch {
    pub fn new(pool: PgPool) -> Self {
        Self {
            reader: pool.clone(),
            has_replica: false,
            replica_replay: Arc::new(AtomicU64::new(0)),
            pool,
            hnsw: crate::hnsw::HnswParams::from_env(),
            model_tables: std::sync::Arc::default(),
            read_routing: Arc::new(SearchReadMetrics::default()),
        }
    }

    /// Route search reads to `reader` once it has caught up to the request's
    /// `Maidan-Consistency-Token` (Cluster 271) — the search-side twin of the
    /// store's replica routing, sharing the same task-local + decision via
    /// [`maidan_store::postgres::replica_route`]. Writes (embedding upserts, DDL,
    /// reindex) always stay on the primary. Spawns the replica-LSN poller.
    pub fn with_replica_reader(pool: PgPool, reader: PgPool) -> Self {
        let replica_replay = Arc::new(AtomicU64::new(0));
        spawn_replica_lsn_poller(reader.clone(), replica_replay.clone());
        Self {
            reader,
            has_replica: true,
            replica_replay,
            pool: pool.clone(),
            hnsw: crate::hnsw::HnswParams::from_env(),
            model_tables: std::sync::Arc::default(),
            read_routing: Arc::new(SearchReadMetrics::default()),
        }
    }

    /// Override the HNSW tuning params (otherwise read from the environment).
    pub fn with_hnsw(mut self, hnsw: crate::hnsw::HnswParams) -> Self {
        self.hnsw = hnsw;
        self
    }

    /// Read-routing counters for the `maidan_search_replica_reads_total` metric
    /// (Cluster 272). The server snapshots this on its metrics tick.
    pub fn read_routing_metrics(&self) -> Arc<SearchReadMetrics> {
        self.read_routing.clone()
    }

    /// The pool a search read should use, honoring the current request's
    /// read-consistency scope (Cluster 271): the replica once its cached replay LSN
    /// has reached the request's token (or when there is no causality requirement),
    /// otherwise the primary. Mirrors `PostgresStore::read_pool`.
    fn read_pool(&self) -> &PgPool {
        let cached = Lsn(self.replica_replay.load(Ordering::Relaxed));
        let to_replica = maidan_store::postgres::replica_route(self.has_replica, cached);
        if self.has_replica {
            let counter = if to_replica {
                &self.read_routing.replica
            } else {
                &self.read_routing.primary
            };
            counter.fetch_add(1, Ordering::Relaxed);
        }
        if to_replica {
            &self.reader
        } else {
            &self.pool
        }
    }
}

/// Poll the replica's `pg_last_wal_replay_lsn()` into `replay_cache` on a fixed
/// cadence (Cluster 271), reusing the store's replication helper. A poll error /
/// non-standby result leaves the cache unchanged low → reads route to the primary
/// until the next good poll (fail-safe). The replica-lag gauge is already emitted by
/// the store's poller against the same replica, so this one does not duplicate it.
fn spawn_replica_lsn_poller(reader: PgPool, replay_cache: Arc<AtomicU64>) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(REPLICA_LSN_POLL_INTERVAL);
        loop {
            tick.tick().await;
            if let Ok(Some(replay)) =
                maidan_store::postgres::replication::replica_replay_lsn(&reader).await
            {
                replay_cache.store(replay.0, Ordering::Relaxed);
            }
        }
    });
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
        // RBAC pre-filter (Cluster 200): exclude denied channels. An empty array
        // makes `<> ALL($8)` vacuously true, so no dynamic SQL is needed.
        let deny = deny_channel_uuids(filters);

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
              AND t.channel_id <> ALL($8)
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
        .bind(deny)
        .fetch_all(self.read_pool())
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
        // Cache hit skips ensure_model_postgres's SELECT + create-checks; the
        // guard is dropped before the await so the lock is never held across it.
        let cached = {
            let guard = self
                .model_tables
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            guard.get(model).cloned()
        };
        let table = match cached {
            Some(t) => t,
            None => {
                let t = embedding_tables::ensure_model_postgres(
                    &self.pool,
                    model,
                    embedding.len(),
                    self.hnsw,
                )
                .await?;
                self.model_tables
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .insert(model.to_string(), t.clone());
                t
            }
        };
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
        // A read: resolve the model table AND run the query against the same pool
        // (the replica once caught up to the request token, else the primary) so the
        // table lookup and the query never disagree about what is replicated.
        let pool = self.read_pool();
        let Some((table, registered_dim)) =
            embedding_tables::resolve_table_postgres(pool, model).await?
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
        let deny = deny_channel_uuids(filters);

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
              AND t.channel_id <> ALL($7)
            ORDER BY e.embedding <=> $2
            LIMIT $3
            "#
        );
        let query = sqlx::query(&sql)
            .bind(workspace_id.0)
            .bind(vector)
            .bind(limit)
            .bind(author_id)
            .bind(channel_id)
            .bind(author_kind)
            .bind(deny);
        // When `ef_search` is configured, set it for this query only via a
        // transaction-scoped `SET LOCAL` (pooled connections are reused, so a
        // session-level SET would leak to other queries).
        let rows = if let Some(ef) = self.hnsw.ef_search {
            let mut tx = pool.begin().await?;
            sqlx::query(&format!("SET LOCAL hnsw.ef_search = {ef}"))
                .execute(&mut *tx)
                .await?;
            let rows = query.fetch_all(&mut *tx).await?;
            tx.commit().await?;
            rows
        } else {
            query.fetch_all(pool).await?
        };

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

    async fn ensure_model(&self, provider: &dyn EmbeddingProvider) -> Result<(), SearchError> {
        embedding_tables::ensure_model_postgres(
            &self.pool,
            provider.model_name(),
            provider.dimension(),
            self.hnsw,
        )
        .await?;
        Ok(())
    }
}

/// The RBAC deny-channel set as raw UUIDs for a `<> ALL($n)` array bind
/// (Cluster 200). An empty vec makes the clause vacuously true.
fn deny_channel_uuids(filters: &SearchFilters) -> Vec<Uuid> {
    filters.deny_channels.iter().map(|c| c.0).collect()
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

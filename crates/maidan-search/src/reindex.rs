//! Re-embed all live messages for the active provider model.

use std::sync::Arc;

use maidan_types::{MessageId, WorkspaceId};
use sqlx::{PgPool, SqlitePool};
use uuid::Uuid;

use crate::embedding_provider::{embed_batch_off_worker, EmbeddingProvider};
use crate::embedding_tables;
use crate::error::SearchError;
use crate::Search;
use maidan_store::usage_meters::UsageMeterBuffer;
use maidan_types::UsageMeter;

#[derive(Debug, Clone, Default)]
pub struct ReindexReport {
    pub processed: u64,
    pub failed: u64,
}

struct MessageRow {
    id: MessageId,
    body: String,
    workspace_id: Option<WorkspaceId>,
}

pub async fn reindex_postgres(
    pool: &PgPool,
    search: &dyn Search,
    provider: &Arc<dyn EmbeddingProvider>,
    workspace_id: Option<WorkspaceId>,
) -> Result<ReindexReport, SearchError> {
    embedding_tables::ensure_model_postgres(
        pool,
        provider.model_name(),
        provider.dimension(),
        crate::hnsw::HnswParams::from_env(),
    )
    .await?;

    let rows = fetch_messages_postgres(pool, workspace_id).await?;
    reindex_rows(search, provider, rows, None).await
}

pub async fn reindex_sqlite(
    pool: &SqlitePool,
    search: &dyn Search,
    provider: &Arc<dyn EmbeddingProvider>,
    workspace_id: Option<WorkspaceId>,
) -> Result<ReindexReport, SearchError> {
    embedding_tables::ensure_model_sqlite(pool, provider.model_name(), provider.dimension())
        .await?;

    let rows = fetch_messages_sqlite(pool, workspace_id).await?;
    reindex_rows(search, provider, rows, None).await
}

/// Session advisory-lock key for the repair sweep, so that of several replicas
/// only one embeds the same missing messages at a time. Any constant works.
const EMBED_REPAIR_LOCK: i64 = 0x6d61_6964_656d_6272;

/// Embed up to `limit` live messages lacking an embedding for the provider's
/// model, newest first. Returns an empty report without doing anything when
/// another replica holds the repair lock.
///
/// With `meters`, a workspace cut off on `embedding_calls` is skipped, and
/// every message taken on is counted to its workspace's `embedding_calls`,
/// as the live indexer does.
pub async fn embed_missing_postgres(
    pool: &PgPool,
    search: &dyn Search,
    provider: &Arc<dyn EmbeddingProvider>,
    limit: i64,
    meters: Option<&UsageMeterBuffer>,
) -> Result<ReindexReport, SearchError> {
    let Some((table, _)) =
        embedding_tables::resolve_table_postgres(pool, provider.model_name()).await?
    else {
        // The model is registered at startup; until then there is no table
        // for anything to be missing from.
        return Ok(ReindexReport::default());
    };
    // The name comes from the registry and is spliced into SQL.
    if !table.starts_with("maidan_emb_")
        || !table.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Err(SearchError::InvalidQuery(format!(
            "invalid embedding table name in registry: {table}"
        )));
    }
    let mut lock = pool.acquire().await?;
    let held: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock($1)")
        .bind(EMBED_REPAIR_LOCK)
        .fetch_one(&mut *lock)
        .await?;
    if !held {
        return Ok(ReindexReport::default());
    }
    let sql = format!(
        r#"
        SELECT m.id, m.body, c.workspace_id
        FROM maidan_messages m
        JOIN maidan_threads t ON t.id = m.thread_id
        JOIN maidan_channels c ON c.id = t.channel_id
        LEFT JOIN {table} e ON e.message_id = m.id
        WHERE e.message_id IS NULL AND m.tombstoned_at IS NULL
          AND NOT (c.workspace_id = ANY($2))
        ORDER BY m.posted_at DESC
        LIMIT $1
        "#
    );
    let cut_off: Vec<Uuid> = meters
        .map(|m| m.cut_off_workspaces(UsageMeter::EmbeddingCalls))
        .unwrap_or_default()
        .into_iter()
        .map(|w| w.0)
        .collect();
    let rows = sqlx::query_as::<_, (Uuid, String, Uuid)>(&sql)
        .bind(limit)
        .bind(cut_off)
        .fetch_all(pool)
        .await;
    let report = match rows {
        Ok(rows) => {
            let rows = rows
                .into_iter()
                .map(|(id, body, workspace_id)| MessageRow {
                    id: MessageId(id),
                    body,
                    workspace_id: Some(WorkspaceId(workspace_id)),
                })
                .collect();
            reindex_rows(search, provider, rows, meters).await
        }
        Err(err) => Err(err.into()),
    };
    sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(EMBED_REPAIR_LOCK)
        .execute(&mut *lock)
        .await?;
    report
}

/// Backfill embeds in batches via [`EmbeddingProvider::embed_batch`] so a
/// remote provider issues one request per chunk instead of one per message.
/// Tuned for throughput, not latency — backfill runs on its own task
/// (`reindex_ops`), never the live indexer queue, so live indexing stays fresh.
const REINDEX_BATCH: usize = 32;

async fn reindex_rows(
    search: &dyn Search,
    provider: &Arc<dyn EmbeddingProvider>,
    rows: Vec<MessageRow>,
    meters: Option<&UsageMeterBuffer>,
) -> Result<ReindexReport, SearchError> {
    let mut report = ReindexReport::default();
    let model = provider.model_name();
    for chunk in rows.chunks(REINDEX_BATCH) {
        // The cutoff can land between the query and this chunk, so it is
        // checked again here; what is taken on is counted before the provider
        // call, as the live indexer counts it.
        let chunk: Vec<&MessageRow> = chunk
            .iter()
            .filter(|row| match (meters, row.workspace_id) {
                (Some(m), Some(w)) => !m.is_cut_off(w, UsageMeter::EmbeddingCalls),
                _ => true,
            })
            .collect();
        if chunk.is_empty() {
            continue;
        }
        if let Some(m) = meters {
            for row in &chunk {
                if let Some(w) = row.workspace_id {
                    m.record(w, UsageMeter::EmbeddingCalls, 1);
                }
            }
        }
        let bodies = chunk.iter().map(|r| r.body.clone()).collect();
        let embeddings = match embed_batch_off_worker(provider, bodies).await {
            Ok(v) => v,
            Err(err) => {
                report.failed += chunk.len() as u64;
                tracing::warn!(batch = chunk.len(), error = %err, "reindex embed batch failed");
                continue;
            }
        };
        for (row, embedding) in chunk.iter().zip(embeddings.iter()) {
            match search.upsert_embedding(row.id, model, embedding).await {
                Ok(()) => report.processed += 1,
                Err(err) => {
                    report.failed += 1;
                    tracing::warn!(message_id = %row.id, error = %err, "reindex upsert failed");
                }
            }
        }
        // Cooperative: let the live indexer (and everything else) make
        // progress between chunks of a large-workspace backfill.
        tokio::task::yield_now().await;
    }
    Ok(report)
}

async fn fetch_messages_postgres(
    pool: &PgPool,
    workspace_id: Option<WorkspaceId>,
) -> Result<Vec<MessageRow>, SearchError> {
    let wid = workspace_id.map(|w| w.0);
    let rows = sqlx::query_as::<_, (Uuid, String)>(
        r#"
        SELECT m.id, m.body
        FROM maidan_messages m
        JOIN maidan_threads t ON t.id = m.thread_id
        JOIN maidan_channels c ON c.id = t.channel_id
        WHERE m.tombstoned_at IS NULL
          AND ($1::uuid IS NULL OR c.workspace_id = $1)
        ORDER BY m.posted_at
        "#,
    )
    .bind(wid)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, body)| MessageRow {
            id: MessageId(id),
            body,
            workspace_id,
        })
        .collect())
}

async fn fetch_messages_sqlite(
    pool: &SqlitePool,
    workspace_id: Option<WorkspaceId>,
) -> Result<Vec<MessageRow>, SearchError> {
    let wid = workspace_id.map(|w| w.0);
    let rows = sqlx::query_as::<_, (Uuid, String)>(
        r#"
        SELECT m.id, m.body
        FROM maidan_messages m
        JOIN maidan_threads t ON t.id = m.thread_id
        JOIN maidan_channels c ON c.id = t.channel_id
        WHERE m.tombstoned_at IS NULL
          AND (? IS NULL OR c.workspace_id = ?)
        ORDER BY m.posted_at
        "#,
    )
    .bind(wid)
    .bind(wid)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, body)| MessageRow {
            id: MessageId(id),
            body,
            workspace_id,
        })
        .collect())
}

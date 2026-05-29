//! Re-embed all live messages for the active provider model (Cluster 47).

use maidan_types::{MessageId, WorkspaceId};
use sqlx::{PgPool, SqlitePool};
use uuid::Uuid;

use crate::embedding_provider::EmbeddingProvider;
use crate::embedding_tables;
use crate::error::SearchError;
use crate::Search;

#[derive(Debug, Clone, Default)]
pub struct ReindexReport {
    pub processed: u64,
    pub failed: u64,
}

struct MessageRow {
    id: MessageId,
    body: String,
}

pub async fn reindex_postgres(
    pool: &PgPool,
    search: &dyn Search,
    provider: &dyn EmbeddingProvider,
    workspace_id: Option<WorkspaceId>,
) -> Result<ReindexReport, SearchError> {
    embedding_tables::ensure_model_postgres(pool, provider.model_name(), provider.dimension())
        .await?;

    let rows = fetch_messages_postgres(pool, workspace_id).await?;
    reindex_rows(search, provider, rows).await
}

pub async fn reindex_sqlite(
    pool: &SqlitePool,
    search: &dyn Search,
    provider: &dyn EmbeddingProvider,
    workspace_id: Option<WorkspaceId>,
) -> Result<ReindexReport, SearchError> {
    embedding_tables::ensure_model_sqlite(pool, provider.model_name(), provider.dimension())
        .await?;

    let rows = fetch_messages_sqlite(pool, workspace_id).await?;
    reindex_rows(search, provider, rows).await
}

async fn reindex_rows(
    search: &dyn Search,
    provider: &dyn EmbeddingProvider,
    rows: Vec<MessageRow>,
) -> Result<ReindexReport, SearchError> {
    let mut report = ReindexReport::default();
    let model = provider.model_name();
    for row in rows {
        let embedding = match provider.embed(&row.body) {
            Ok(v) => v,
            Err(err) => {
                report.failed += 1;
                tracing::warn!(message_id = %row.id, error = %err, "reindex embed failed");
                continue;
            }
        };
        match search.upsert_embedding(row.id, model, &embedding).await {
            Ok(()) => report.processed += 1,
            Err(err) => {
                report.failed += 1;
                tracing::warn!(message_id = %row.id, error = %err, "reindex upsert failed");
            }
        }
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
        })
        .collect())
}

async fn fetch_messages_sqlite(
    pool: &SqlitePool,
    workspace_id: Option<WorkspaceId>,
) -> Result<Vec<MessageRow>, SearchError> {
    let wid = workspace_id.map(|w| w.0.to_string());
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
    .bind(wid.as_deref())
    .bind(wid.as_deref())
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, body)| MessageRow {
            id: MessageId(id),
            body,
        })
        .collect())
}

use chrono::Utc;
use maidan_types::{WorkspaceId, WorkspacePurgeResult};
use sqlx::SqlitePool;

use crate::embeddings_purge;
use crate::error::StoreError;

pub async fn purge(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
) -> Result<WorkspacePurgeResult, StoreError> {
    let now = Utc::now();

    let embeddings_removed =
        embeddings_purge::purge_workspace_embeddings_sqlite(pool, workspace_id).await?;

    let references_removed = sqlx::query(
        "DELETE FROM maidan_references
         WHERE (src_kind = 'message' AND src_id IN (
                 SELECT m.id FROM maidan_messages m
                 INNER JOIN maidan_threads t ON m.thread_id = t.id
                 INNER JOIN maidan_channels c ON t.channel_id = c.id
                 WHERE c.workspace_id = ?
               ))
            OR (dst_kind = 'message' AND dst_id IN (
                 SELECT m.id FROM maidan_messages m
                 INNER JOIN maidan_threads t ON m.thread_id = t.id
                 INNER JOIN maidan_channels c ON t.channel_id = c.id
                 WHERE c.workspace_id = ?
               ))
            OR (src_kind = 'thread' AND src_id IN (
                 SELECT t.id FROM maidan_threads t
                 INNER JOIN maidan_channels c ON t.channel_id = c.id
                 WHERE c.workspace_id = ?
               ))
            OR (dst_kind = 'thread' AND dst_id IN (
                 SELECT t.id FROM maidan_threads t
                 INNER JOIN maidan_channels c ON t.channel_id = c.id
                 WHERE c.workspace_id = ?
               ))",
    )
    .bind(workspace_id.0)
    .bind(workspace_id.0)
    .bind(workspace_id.0)
    .bind(workspace_id.0)
    .execute(pool)
    .await?;

    let tombstone = sqlx::query(
        "UPDATE maidan_messages SET tombstoned_at = ?, body = '', content = NULL
         WHERE tombstoned_at IS NULL
           AND thread_id IN (
             SELECT t.id FROM maidan_threads t
             INNER JOIN maidan_channels c ON t.channel_id = c.id
             WHERE c.workspace_id = ?
           )",
    )
    .bind(now)
    .bind(workspace_id.0)
    .execute(pool)
    .await?;

    let purge = sqlx::query(
        "DELETE FROM maidan_messages
         WHERE tombstoned_at IS NOT NULL
           AND thread_id IN (
             SELECT t.id FROM maidan_threads t
             INNER JOIN maidan_channels c ON t.channel_id = c.id
             WHERE c.workspace_id = ?
           )",
    )
    .bind(workspace_id.0)
    .execute(pool)
    .await?;

    let api_tokens_revoked = sqlx::query(
        "UPDATE maidan_api_tokens SET revoked_at = ?
         WHERE workspace_id = ? AND revoked_at IS NULL",
    )
    .bind(now)
    .bind(workspace_id.0)
    .execute(pool)
    .await?;

    let events_removed = sqlx::query("DELETE FROM maidan_events WHERE workspace_id = ?")
        .bind(workspace_id.0)
        .execute(pool)
        .await?;

    let artifact_shas: Vec<String> = sqlx::query_scalar(
        "SELECT sha256 FROM maidan_artifacts
         WHERE uploaded_by IN (SELECT id FROM maidan_members WHERE workspace_id = ?)",
    )
    .bind(workspace_id.0)
    .fetch_all(pool)
    .await?;

    let artifacts_removed = sqlx::query(
        "DELETE FROM maidan_artifacts
         WHERE uploaded_by IN (SELECT id FROM maidan_members WHERE workspace_id = ?)",
    )
    .bind(workspace_id.0)
    .execute(pool)
    .await?;

    Ok(WorkspacePurgeResult {
        workspace_id,
        messages_tombstoned: tombstone.rows_affected(),
        messages_purged: purge.rows_affected(),
        embeddings_removed: embeddings_removed as u64,
        references_removed: references_removed.rows_affected(),
        api_tokens_revoked: api_tokens_revoked.rows_affected(),
        events_removed: events_removed.rows_affected(),
        artifacts_removed: artifacts_removed.rows_affected(),
        artifact_shas,
        occurred_at: now,
    })
}

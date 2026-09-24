use chrono::Utc;
use maidan_types::{WorkspaceId, WorkspacePurgeResult};
use sqlx::{PgConnection, PgPool};

use crate::embeddings_purge;
use crate::error::StoreError;

pub async fn purge(
    pool: &PgPool,
    workspace_id: WorkspaceId,
) -> Result<WorkspacePurgeResult, StoreError> {
    let mut conn = pool.acquire().await?;
    purge_on(&mut conn, workspace_id).await
}

/// Everything the purge removes goes in one transaction, which refuses a held
/// workspace — the check and the destruction cannot be separated by a hold
/// placed in between.
pub(crate) async fn purge_on(
    conn: &mut PgConnection,
    workspace_id: WorkspaceId,
) -> Result<WorkspacePurgeResult, StoreError> {
    let mut tx = sqlx::Connection::begin(&mut *conn).await?;
    super::legal_hold::refuse_if_held(&mut tx, workspace_id).await?;
    let embeddings_removed =
        embeddings_purge::purge_workspace_embeddings_postgres(&mut tx, workspace_id).await?;

    let references_removed = sqlx::query(
        "DELETE FROM maidan_references r
         WHERE (r.src_kind = 'message' AND r.src_id IN (
                 SELECT m.id FROM maidan_messages m
                 INNER JOIN maidan_threads t ON m.thread_id = t.id
                 INNER JOIN maidan_channels c ON t.channel_id = c.id
                 WHERE c.workspace_id = $1
               ))
            OR (r.dst_kind = 'message' AND r.dst_id IN (
                 SELECT m.id FROM maidan_messages m
                 INNER JOIN maidan_threads t ON m.thread_id = t.id
                 INNER JOIN maidan_channels c ON t.channel_id = c.id
                 WHERE c.workspace_id = $1
               ))
            OR (r.src_kind = 'thread' AND r.src_id IN (
                 SELECT t.id FROM maidan_threads t
                 INNER JOIN maidan_channels c ON t.channel_id = c.id
                 WHERE c.workspace_id = $1
               ))
            OR (r.dst_kind = 'thread' AND r.dst_id IN (
                 SELECT t.id FROM maidan_threads t
                 INNER JOIN maidan_channels c ON t.channel_id = c.id
                 WHERE c.workspace_id = $1
               ))",
    )
    .bind(workspace_id.0)
    .execute(&mut *tx)
    .await?;

    let tombstone = sqlx::query(
        "UPDATE maidan_messages SET tombstoned_at = NOW(), body = '', content = NULL
         WHERE tombstoned_at IS NULL
           AND thread_id IN (
             SELECT t.id FROM maidan_threads t
             INNER JOIN maidan_channels c ON t.channel_id = c.id
             WHERE c.workspace_id = $1
           )",
    )
    .bind(workspace_id.0)
    .execute(&mut *tx)
    .await?;

    let purge = sqlx::query(
        "DELETE FROM maidan_messages
         WHERE tombstoned_at IS NOT NULL
           AND thread_id IN (
             SELECT t.id FROM maidan_threads t
             INNER JOIN maidan_channels c ON t.channel_id = c.id
             WHERE c.workspace_id = $1
           )",
    )
    .bind(workspace_id.0)
    .execute(&mut *tx)
    .await?;

    let api_tokens_revoked = sqlx::query(
        "UPDATE maidan_api_tokens SET revoked_at = NOW()
         WHERE workspace_id = $1 AND revoked_at IS NULL",
    )
    .bind(workspace_id.0)
    .execute(&mut *tx)
    .await?;

    let events_removed = sqlx::query("DELETE FROM maidan_events WHERE workspace_id = $1")
        .bind(workspace_id.0)
        .execute(&mut *tx)
        .await?;

    // Artifacts are content-addressed and shared: one row and one blob per
    // sha, whoever uploaded it first, and per-workspace access lives in
    // `maidan_artifact_refs` (Cluster 204). So a workspace's purge drops its own
    // references and destroys an artifact only when no other workspace still
    // references it. Deleting by `uploaded_by` — as this used to — destroyed
    // another tenant's content whenever it had uploaded the same bytes after
    // this workspace did, and left this workspace's copy behind when the other
    // tenant had uploaded first.
    let referenced: Vec<String> = sqlx::query_scalar(
        "DELETE FROM maidan_artifact_refs WHERE workspace_id = $1 RETURNING sha256",
    )
    .bind(workspace_id.0)
    .fetch_all(&mut *tx)
    .await?;
    let mut artifact_shas = Vec::new();
    for sha in referenced {
        // Lock the row first, so an upload of the same bytes racing this purge
        // either lands its reference before the check below or waits for it.
        sqlx::query("SELECT 1 FROM maidan_artifacts WHERE sha256 = $1 FOR UPDATE")
            .bind(&sha)
            .fetch_optional(&mut *tx)
            .await?;
        let removed = sqlx::query(
            "DELETE FROM maidan_artifacts WHERE sha256 = $1
             AND NOT EXISTS (SELECT 1 FROM maidan_artifact_refs WHERE sha256 = $1)",
        )
        .bind(&sha)
        .execute(&mut *tx)
        .await?;
        if removed.rows_affected() > 0 {
            artifact_shas.push(sha);
        }
    }
    // An artifact nobody references is readable by nobody; if this workspace's
    // members uploaded it, it goes too, so an erasure leaves none of their
    // content behind.
    let unreferenced: Vec<String> = sqlx::query_scalar(
        "DELETE FROM maidan_artifacts
         WHERE uploaded_by IN (SELECT id FROM maidan_members WHERE workspace_id = $1)
           AND NOT EXISTS (
             SELECT 1 FROM maidan_artifact_refs r WHERE r.sha256 = maidan_artifacts.sha256
           )
         RETURNING sha256",
    )
    .bind(workspace_id.0)
    .fetch_all(&mut *tx)
    .await?;
    artifact_shas.extend(unreferenced);
    tx.commit().await?;
    let artifacts_removed = artifact_shas.len() as u64;

    Ok(WorkspacePurgeResult {
        workspace_id,
        messages_tombstoned: tombstone.rows_affected(),
        messages_purged: purge.rows_affected(),
        embeddings_removed: embeddings_removed as u64,
        references_removed: references_removed.rows_affected(),
        api_tokens_revoked: api_tokens_revoked.rows_affected(),
        events_removed: events_removed.rows_affected(),
        artifacts_removed,
        artifact_shas,
        occurred_at: Utc::now(),
    })
}

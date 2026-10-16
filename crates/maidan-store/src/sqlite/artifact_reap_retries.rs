//! Orphaned artifact bytes waiting for another delete. See
//! `ArtifactMetaStore::queue_blob_reap`.

use maidan_types::WorkspaceId;
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::error::StoreError;
use crate::PendingBlobReap;

pub async fn queue(
    pool: &SqlitePool,
    sha256: &str,
    workspace_id: WorkspaceId,
    error: &str,
) -> Result<(), StoreError> {
    // A sha already queued keeps its attempts and its next try; a second
    // erase orphaning it again only says who did and why it failed.
    sqlx::query(
        "INSERT INTO maidan_artifact_reap_retries (sha256, workspace_id, last_error)
         VALUES (?1, ?2, ?3)
         ON CONFLICT (sha256) DO UPDATE
             SET workspace_id = excluded.workspace_id, last_error = excluded.last_error",
    )
    .bind(sha256)
    .bind(workspace_id.0)
    .bind(error)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn due(pool: &SqlitePool, limit: i64) -> Result<Vec<PendingBlobReap>, StoreError> {
    let rows = sqlx::query(
        "SELECT sha256, workspace_id, attempts, last_error
         FROM maidan_artifact_reap_retries
         WHERE next_attempt_at <= strftime('%Y-%m-%dT%H:%M:%fZ', 'now')
         ORDER BY next_attempt_at, sha256
         LIMIT ?",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;
    rows.iter()
        .map(|row| {
            Ok(PendingBlobReap {
                sha256: row.try_get("sha256")?,
                workspace_id: WorkspaceId(row.try_get::<Uuid, _>("workspace_id")?),
                attempts: row.try_get("attempts")?,
                last_error: row.try_get("last_error")?,
            })
        })
        .collect()
}

pub async fn defer(
    pool: &SqlitePool,
    sha256: &str,
    error: &str,
    delay_secs: i64,
) -> Result<(), StoreError> {
    sqlx::query(
        "UPDATE maidan_artifact_reap_retries
         SET attempts = attempts + 1, last_error = ?2,
             next_attempt_at = strftime('%Y-%m-%dT%H:%M:%fZ', 'now', ?3)
         WHERE sha256 = ?1",
    )
    .bind(sha256)
    .bind(error)
    .bind(format!("+{delay_secs} seconds"))
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn settle(pool: &SqlitePool, sha256: &str) -> Result<(), StoreError> {
    sqlx::query("DELETE FROM maidan_artifact_reap_retries WHERE sha256 = ?")
        .bind(sha256)
        .execute(pool)
        .await?;
    Ok(())
}

//! Orphaned artifact bytes waiting for another delete; see the SQLite twin.

use maidan_types::WorkspaceId;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::error::StoreError;
use crate::PendingBlobReap;

pub async fn queue(
    pool: &PgPool,
    sha256: &str,
    workspace_id: WorkspaceId,
    error: &str,
) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO maidan_artifact_reap_retries (sha256, workspace_id, last_error)
         VALUES ($1, $2, $3)
         ON CONFLICT (sha256) DO UPDATE
             SET workspace_id = EXCLUDED.workspace_id, last_error = EXCLUDED.last_error",
    )
    .bind(sha256)
    .bind(workspace_id.0)
    .bind(error)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn due(pool: &PgPool, limit: i64) -> Result<Vec<PendingBlobReap>, StoreError> {
    let rows = sqlx::query(
        "SELECT sha256, workspace_id, attempts, last_error
         FROM maidan_artifact_reap_retries
         WHERE next_attempt_at <= now()
         ORDER BY next_attempt_at, sha256
         LIMIT $1",
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
    pool: &PgPool,
    sha256: &str,
    error: &str,
    delay_secs: i64,
) -> Result<(), StoreError> {
    sqlx::query(
        "UPDATE maidan_artifact_reap_retries
         SET attempts = attempts + 1, last_error = $2,
             next_attempt_at = now() + make_interval(secs => $3)
         WHERE sha256 = $1",
    )
    .bind(sha256)
    .bind(error)
    .bind(delay_secs as f64)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn settle(pool: &PgPool, sha256: &str) -> Result<(), StoreError> {
    sqlx::query("DELETE FROM maidan_artifact_reap_retries WHERE sha256 = $1")
        .bind(sha256)
        .execute(pool)
        .await?;
    Ok(())
}

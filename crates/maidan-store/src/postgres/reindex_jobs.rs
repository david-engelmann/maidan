//! Postgres-backed embedding reindex jobs (Cluster 104.0.3).

use chrono::{DateTime, Utc};
use maidan_types::{ReindexJob, ReindexJobStatus};
use sqlx::PgPool;

use crate::error::StoreError;

/// `(job_id, status, workspace_id, embedding_model, processed, failed, error,
/// started_at, finished_at)`.
type JobRow = (
    uuid::Uuid,
    String,
    Option<uuid::Uuid>,
    String,
    Option<i64>,
    Option<i64>,
    Option<String>,
    DateTime<Utc>,
    Option<DateTime<Utc>>,
);

pub async fn upsert(pool: &PgPool, job: ReindexJob) -> Result<(), StoreError> {
    sqlx::query(
        "INSERT INTO maidan_reindex_jobs
            (job_id, status, workspace_id, embedding_model,
             processed, failed, error, started_at, finished_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
         ON CONFLICT (job_id) DO UPDATE SET
            status = EXCLUDED.status,
            processed = EXCLUDED.processed,
            failed = EXCLUDED.failed,
            error = EXCLUDED.error,
            finished_at = EXCLUDED.finished_at",
    )
    .bind(job.job_id)
    .bind(status_str(&job.status))
    .bind(job.workspace_id)
    .bind(job.embedding_model)
    .bind(job.processed.map(|v| v as i64))
    .bind(job.failed.map(|v| v as i64))
    .bind(job.error)
    .bind(job.started_at)
    .bind(job.finished_at)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get(pool: &PgPool, job_id: uuid::Uuid) -> Result<Option<ReindexJob>, StoreError> {
    let row: Option<JobRow> = sqlx::query_as(
        "SELECT job_id, status, workspace_id, embedding_model,
                processed, failed, error, started_at, finished_at
         FROM maidan_reindex_jobs WHERE job_id = $1",
    )
    .bind(job_id)
    .fetch_optional(pool)
    .await?;
    row.map(row_to_job).transpose()
}

fn status_str(status: &ReindexJobStatus) -> &'static str {
    match status {
        ReindexJobStatus::Running => "running",
        ReindexJobStatus::Completed => "completed",
        ReindexJobStatus::Failed => "failed",
    }
}

fn parse_status(s: &str) -> Result<ReindexJobStatus, StoreError> {
    match s {
        "running" => Ok(ReindexJobStatus::Running),
        "completed" => Ok(ReindexJobStatus::Completed),
        "failed" => Ok(ReindexJobStatus::Failed),
        other => Err(StoreError::InvalidInput(format!(
            "unknown reindex status: {other}"
        ))),
    }
}

fn row_to_job(row: JobRow) -> Result<ReindexJob, StoreError> {
    let (
        job_id,
        status,
        workspace_id,
        embedding_model,
        processed,
        failed,
        error,
        started,
        finished,
    ) = row;
    Ok(ReindexJob {
        job_id,
        status: parse_status(&status)?,
        workspace_id,
        embedding_model,
        processed: processed.map(|v| v as u64),
        failed: failed.map(|v| v as u64),
        error,
        started_at: started,
        finished_at: finished,
    })
}

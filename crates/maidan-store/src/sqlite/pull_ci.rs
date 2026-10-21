//! CI on tracked pull requests on SQLite; see the Postgres twin.

use chrono::{DateTime, Utc};
use maidan_types::{CiStatus, NewPullCi, PostedNotice, PullCi, ThreadId, WorkspaceId};
use sqlx::types::Json;
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use super::change_metrics::insert_notice_in_tx;
use crate::error::StoreError;

/// The thread `repo`#`pull_number` is tracked on in `workspace_id`; `None`
/// when no workspace tracks it, and a conflict when another one does.
pub async fn tracked_thread(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    repo: &str,
    pull_number: i64,
) -> Result<Option<ThreadId>, StoreError> {
    let repo = repo.to_ascii_lowercase();
    let row = sqlx::query(
        "SELECT workspace_id, thread_id FROM maidan_change_pulls
         WHERE repo = ?1 AND pull_number = ?2",
    )
    .bind(&repo)
    .bind(pull_number)
    .fetch_optional(pool)
    .await?;
    match row {
        None => Ok(None),
        Some(row) if row.get::<Uuid, _>("workspace_id") == workspace_id.0 => {
            Ok(Some(ThreadId(row.get("thread_id"))))
        }
        Some(_) => Err(StoreError::Conflict(format!(
            "{repo}#{pull_number} is already tracked on another task"
        ))),
    }
}

/// The last read of a tracked pull request's checks in `workspace_id`.
pub async fn get(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    repo: &str,
    pull_number: i64,
) -> Result<Option<PullCi>, StoreError> {
    let row = sqlx::query(
        "SELECT repo, pull_number, head_sha, status, failing_jobs, log_tail, updated_at
         FROM maidan_pull_ci WHERE workspace_id = ?1 AND repo = ?2 AND pull_number = ?3",
    )
    .bind(workspace_id.0)
    .bind(repo.to_ascii_lowercase())
    .bind(pull_number)
    .fetch_optional(pool)
    .await?;
    row.as_ref().map(ci_from).transpose()
}

/// The last reads of the checks on every pull request tracked on a thread.
pub async fn list_for_thread(
    pool: &SqlitePool,
    thread_id: ThreadId,
) -> Result<Vec<PullCi>, StoreError> {
    let rows = sqlx::query(
        "SELECT c.repo, c.pull_number, c.head_sha, c.status, c.failing_jobs, c.log_tail, c.updated_at
         FROM maidan_pull_ci c
         JOIN maidan_change_pulls p
           ON p.repo = c.repo AND p.pull_number = c.pull_number AND p.workspace_id = c.workspace_id
         WHERE p.thread_id = ?1
         ORDER BY c.repo, c.pull_number",
    )
    .bind(thread_id.0)
    .fetch_all(pool)
    .await?;
    rows.iter().map(ci_from).collect()
}

/// Record a read of a tracked pull request's checks and the notices it owes
/// its thread, in one transaction. Each notice's key says it once, so a
/// second replica's read of the same head records nothing new. Nothing when
/// the pull request is not tracked in `new.workspace_id`.
pub async fn record(pool: &SqlitePool, new: NewPullCi) -> Result<Vec<PostedNotice>, StoreError> {
    let repo = new.repo.to_ascii_lowercase();
    let mut tx = pool.begin().await?;
    let thread: Option<Uuid> = sqlx::query_scalar(
        "SELECT thread_id FROM maidan_change_pulls
         WHERE workspace_id = ?1 AND repo = ?2 AND pull_number = ?3",
    )
    .bind(new.workspace_id.0)
    .bind(&repo)
    .bind(new.pull_number)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(thread) = thread else {
        return Ok(Vec::new());
    };
    let now = Utc::now();
    sqlx::query(
        "INSERT INTO maidan_pull_ci
             (repo, pull_number, workspace_id, head_sha, status, failing_jobs, log_tail, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
         ON CONFLICT (repo, pull_number) DO UPDATE SET
             head_sha = excluded.head_sha, status = excluded.status,
             failing_jobs = excluded.failing_jobs, log_tail = excluded.log_tail,
             updated_at = excluded.updated_at
         WHERE maidan_pull_ci.workspace_id = excluded.workspace_id",
    )
    .bind(&repo)
    .bind(new.pull_number)
    .bind(new.workspace_id.0)
    .bind(new.head_sha.to_ascii_lowercase())
    .bind(new.status.as_str())
    .bind(Json(&new.failing_jobs))
    .bind(&new.log_tail)
    .bind(now.to_rfc3339())
    .execute(&mut *tx)
    .await?;
    let mut posted = Vec::new();
    for (notice, key) in maidan_types::ci_notices(&new, now) {
        if let Some(p) = insert_notice_in_tx(&mut tx, ThreadId(thread), notice, &key).await? {
            posted.push(p);
        }
    }
    tx.commit().await?;
    Ok(posted)
}

fn ci_from(row: &sqlx::sqlite::SqliteRow) -> Result<PullCi, StoreError> {
    let status: String = row.get("status");
    Ok(PullCi {
        repo: row.get("repo"),
        pull_number: row.get("pull_number"),
        head_sha: row.get("head_sha"),
        status: CiStatus::parse(&status)
            .ok_or_else(|| StoreError::InvalidInput(format!("unknown CI status `{status}`")))?,
        failing_jobs: row.get::<Json<Vec<String>>, _>("failing_jobs").0,
        log_tail: row.get("log_tail"),
        updated_at: row.get::<DateTime<Utc>, _>("updated_at"),
    })
}

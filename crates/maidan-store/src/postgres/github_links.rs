//! GitHub projector issue/PR links: map a GitHub issue/PR to the Maidan
//! channel/thread it projects into. See the SQLite twin.

use sqlx::{PgPool, Row};

use crate::StoreError;
use maidan_types::{
    ChannelId, GithubIssueLink, MemberId, NewGithubIssueLink, ThreadId, WorkspaceId,
};

const COLS: &str =
    "repo, issue_number, workspace_id, channel_id, thread_id, member_id, created_at, disabled_at";

/// Create or replace the link for a GitHub issue/PR (one per repo+number, and —
/// — one per thread).
pub async fn link(pool: &PgPool, new: NewGithubIssueLink) -> Result<GithubIssueLink, StoreError> {
    let row = sqlx::query(&format!(
        "INSERT INTO maidan_github_issue_links
           (repo, issue_number, workspace_id, channel_id, thread_id, member_id, created_at)
         VALUES ($1, $2, $3, $4, $5, $6, now())
         ON CONFLICT (repo, issue_number) DO UPDATE
           SET workspace_id = EXCLUDED.workspace_id, channel_id = EXCLUDED.channel_id,
               thread_id = EXCLUDED.thread_id, member_id = EXCLUDED.member_id,
               disabled_at = NULL
         RETURNING {COLS}"
    ))
    .bind(&new.repo)
    .bind(new.issue_number)
    .bind(new.workspace_id.0)
    .bind(new.channel_id.0)
    .bind(new.thread_id.0)
    .bind(new.member_id.0)
    .fetch_one(pool)
    .await
    .map_err(map_link_err)?;
    Ok(row_to_link(&row))
}

/// The only uniqueness this insert can violate is the one-link-per-thread index
/// — `(repo, issue_number)` is absorbed by the `ON CONFLICT` upsert. Report it
/// as a `Conflict` (SpawnRejected), not an opaque 500.
fn map_link_err(err: sqlx::Error) -> StoreError {
    if let sqlx::Error::Database(ref db) = err {
        if db.is_unique_violation() {
            return StoreError::Conflict(
                "spawn budget: the thread already has a GitHub link (at most one per claim)".into(),
            );
        }
    }
    StoreError::Database(err)
}

pub async fn get(
    pool: &PgPool,
    repo: &str,
    issue_number: i64,
) -> Result<Option<GithubIssueLink>, StoreError> {
    let row = sqlx::query(&format!(
        "SELECT {COLS} FROM maidan_github_issue_links WHERE repo = $1 AND issue_number = $2"
    ))
    .bind(repo)
    .bind(issue_number)
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(row_to_link))
}

/// The egress reverse lookup: the link for a Maidan thread.
pub async fn get_by_thread(
    pool: &PgPool,
    thread_id: ThreadId,
) -> Result<Option<GithubIssueLink>, StoreError> {
    let row = sqlx::query(&format!(
        "SELECT {COLS} FROM maidan_github_issue_links WHERE thread_id = $1 LIMIT 1"
    ))
    .bind(thread_id.0)
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(row_to_link))
}

pub async fn list(
    pool: &PgPool,
    workspace_id: WorkspaceId,
) -> Result<Vec<GithubIssueLink>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT {COLS} FROM maidan_github_issue_links WHERE workspace_id = $1 ORDER BY created_at DESC"
    ))
    .bind(workspace_id.0)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_link).collect())
}

pub async fn unlink(pool: &PgPool, repo: &str, issue_number: i64) -> Result<bool, StoreError> {
    let res =
        sqlx::query("DELETE FROM maidan_github_issue_links WHERE repo = $1 AND issue_number = $2")
            .bind(repo)
            .bind(issue_number)
            .execute(pool)
            .await?;
    Ok(res.rows_affected() > 0)
}

/// Turn egress to this issue/PR off. Idempotent: an already-disabled link keeps
/// its original timestamp. Returns whether this call did the disabling.
pub async fn disable(pool: &PgPool, repo: &str, issue_number: i64) -> Result<bool, StoreError> {
    let res = sqlx::query(
        "UPDATE maidan_github_issue_links SET disabled_at = now()
         WHERE repo = $1 AND issue_number = $2 AND disabled_at IS NULL",
    )
    .bind(repo)
    .bind(issue_number)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

fn row_to_link(row: &sqlx::postgres::PgRow) -> GithubIssueLink {
    GithubIssueLink {
        repo: row.get("repo"),
        issue_number: row.get("issue_number"),
        workspace_id: WorkspaceId(row.get("workspace_id")),
        channel_id: ChannelId(row.get("channel_id")),
        thread_id: ThreadId(row.get("thread_id")),
        member_id: MemberId(row.get("member_id")),
        created_at: row.get("created_at"),
        disabled_at: row.get("disabled_at"),
    }
}

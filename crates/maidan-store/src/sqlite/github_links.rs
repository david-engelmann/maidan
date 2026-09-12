//! GitHub projector issue/PR links (Cluster 311). SQLite twin of the Postgres module.

use chrono::Utc;
use sqlx::{Row, SqlitePool};

use crate::StoreError;
use maidan_types::{
    ChannelId, GithubIssueLink, MemberId, NewGithubIssueLink, ThreadId, WorkspaceId,
};

const COLS: &str =
    "repo, issue_number, workspace_id, channel_id, thread_id, member_id, created_at, disabled_at";

pub async fn link(
    pool: &SqlitePool,
    new: NewGithubIssueLink,
) -> Result<GithubIssueLink, StoreError> {
    let now = Utc::now().to_rfc3339();
    let row = sqlx::query(&format!(
        "INSERT INTO maidan_github_issue_links
           (repo, issue_number, workspace_id, channel_id, thread_id, member_id, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)
         ON CONFLICT (repo, issue_number) DO UPDATE
           SET workspace_id = excluded.workspace_id, channel_id = excluded.channel_id,
               thread_id = excluded.thread_id, member_id = excluded.member_id,
               disabled_at = NULL
         RETURNING {COLS}"
    ))
    .bind(&new.repo)
    .bind(new.issue_number)
    .bind(new.workspace_id.0)
    .bind(new.channel_id.0)
    .bind(new.thread_id.0)
    .bind(new.member_id.0)
    .bind(&now)
    .fetch_one(pool)
    .await
    .map_err(map_link_err)?;
    Ok(row_to_link(&row))
}

/// See the Postgres twin: the only uniqueness this upsert can violate is the
/// one-link-per-thread index (Cluster 376.5).
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
    pool: &SqlitePool,
    repo: &str,
    issue_number: i64,
) -> Result<Option<GithubIssueLink>, StoreError> {
    let row = sqlx::query(&format!(
        "SELECT {COLS} FROM maidan_github_issue_links WHERE repo = ? AND issue_number = ?"
    ))
    .bind(repo)
    .bind(issue_number)
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(row_to_link))
}

pub async fn get_by_thread(
    pool: &SqlitePool,
    thread_id: ThreadId,
) -> Result<Option<GithubIssueLink>, StoreError> {
    let row = sqlx::query(&format!(
        "SELECT {COLS} FROM maidan_github_issue_links WHERE thread_id = ? LIMIT 1"
    ))
    .bind(thread_id.0)
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(row_to_link))
}

pub async fn list(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
) -> Result<Vec<GithubIssueLink>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT {COLS} FROM maidan_github_issue_links WHERE workspace_id = ? ORDER BY created_at DESC"
    ))
    .bind(workspace_id.0)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_link).collect())
}

pub async fn unlink(pool: &SqlitePool, repo: &str, issue_number: i64) -> Result<bool, StoreError> {
    let res =
        sqlx::query("DELETE FROM maidan_github_issue_links WHERE repo = ? AND issue_number = ?")
            .bind(repo)
            .bind(issue_number)
            .execute(pool)
            .await?;
    Ok(res.rows_affected() > 0)
}

pub async fn disable(pool: &SqlitePool, repo: &str, issue_number: i64) -> Result<bool, StoreError> {
    let res = sqlx::query(
        "UPDATE maidan_github_issue_links SET disabled_at = ?
         WHERE repo = ? AND issue_number = ? AND disabled_at IS NULL",
    )
    .bind(Utc::now().to_rfc3339())
    .bind(repo)
    .bind(issue_number)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

fn row_to_link(row: &sqlx::sqlite::SqliteRow) -> GithubIssueLink {
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

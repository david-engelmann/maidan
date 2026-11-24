//! Slack projector channel links. SQLite twin of the Postgres module.

use chrono::Utc;
use sqlx::{Row, SqlitePool};

use crate::StoreError;
use maidan_types::{
    ChannelId, MemberId, NewSlackChannelLink, SlackChannelLink, ThreadId, WorkspaceId,
};

const COLS: &str =
    "slack_channel_id, workspace_id, channel_id, thread_id, member_id, created_at, disabled_at";

pub async fn link(
    pool: &SqlitePool,
    new: NewSlackChannelLink,
) -> Result<SlackChannelLink, StoreError> {
    let now = Utc::now().to_rfc3339();
    let row = sqlx::query(&format!(
        "INSERT INTO maidan_slack_channel_links
           (slack_channel_id, workspace_id, channel_id, thread_id, member_id, created_at)
         VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT (workspace_id, slack_channel_id) DO UPDATE
           SET channel_id = excluded.channel_id, thread_id = excluded.thread_id,
               member_id = excluded.member_id, disabled_at = NULL
         RETURNING {COLS}"
    ))
    .bind(&new.slack_channel_id)
    .bind(new.workspace_id.0)
    .bind(new.channel_id.0)
    .bind(new.thread_id.0)
    .bind(new.member_id.0)
    .bind(&now)
    .fetch_one(pool)
    .await?;
    Ok(row_to_link(&row))
}

/// The workspace's own link for a Slack channel.
pub async fn get(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    slack_channel_id: &str,
) -> Result<Option<SlackChannelLink>, StoreError> {
    let row = sqlx::query(&format!(
        "SELECT {COLS} FROM maidan_slack_channel_links
         WHERE workspace_id = ? AND slack_channel_id = ?"
    ))
    .bind(workspace_id.0)
    .bind(slack_channel_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(row_to_link))
}

/// The links for a Slack channel in the workspaces `scope` reaches, for an
/// inbound delivery.
pub async fn list_for_channel(
    pool: &SqlitePool,
    scope: maidan_types::InboundScope,
    slack_channel_id: &str,
) -> Result<Vec<SlackChannelLink>, StoreError> {
    let sql = format!(
        "SELECT {COLS} FROM maidan_slack_channel_links
         WHERE slack_channel_id = ? AND {} ORDER BY created_at",
        crate::inbound_scope::clause(scope, "workspace_id", "?")
    );
    let query = sqlx::query(&sql).bind(slack_channel_id);
    let query = match crate::inbound_scope::bind(scope) {
        crate::inbound_scope::Bind::Workspace(id) => query.bind(id),
        crate::inbound_scope::Bind::Secret(name) => query.bind(name),
    };
    let rows = query.fetch_all(pool).await?;
    Ok(rows.iter().map(row_to_link).collect())
}

pub async fn get_by_thread(
    pool: &SqlitePool,
    thread_id: ThreadId,
) -> Result<Option<SlackChannelLink>, StoreError> {
    let row = sqlx::query(&format!(
        "SELECT {COLS} FROM maidan_slack_channel_links WHERE thread_id = ? LIMIT 1"
    ))
    .bind(thread_id.0)
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(row_to_link))
}

pub async fn list(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
) -> Result<Vec<SlackChannelLink>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT {COLS} FROM maidan_slack_channel_links WHERE workspace_id = ? ORDER BY created_at DESC"
    ))
    .bind(workspace_id.0)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_link).collect())
}

pub async fn unlink(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    slack_channel_id: &str,
) -> Result<bool, StoreError> {
    let res = sqlx::query(
        "DELETE FROM maidan_slack_channel_links
         WHERE workspace_id = ? AND slack_channel_id = ?",
    )
    .bind(workspace_id.0)
    .bind(slack_channel_id)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

pub async fn disable(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    slack_channel_id: &str,
) -> Result<bool, StoreError> {
    let res = sqlx::query(
        "UPDATE maidan_slack_channel_links SET disabled_at = ?
         WHERE workspace_id = ? AND slack_channel_id = ? AND disabled_at IS NULL",
    )
    .bind(Utc::now().to_rfc3339())
    .bind(workspace_id.0)
    .bind(slack_channel_id)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

fn row_to_link(row: &sqlx::sqlite::SqliteRow) -> SlackChannelLink {
    SlackChannelLink {
        slack_channel_id: row.get("slack_channel_id"),
        workspace_id: WorkspaceId(row.get("workspace_id")),
        channel_id: ChannelId(row.get("channel_id")),
        thread_id: ThreadId(row.get("thread_id")),
        member_id: MemberId(row.get("member_id")),
        created_at: row.get("created_at"),
        disabled_at: row.get("disabled_at"),
    }
}

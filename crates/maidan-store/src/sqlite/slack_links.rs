//! Slack projector channel links (Cluster 308). SQLite twin of the Postgres module.

use chrono::Utc;
use sqlx::{Row, SqlitePool};

use crate::StoreError;
use maidan_types::{
    ChannelId, MemberId, NewSlackChannelLink, SlackChannelLink, ThreadId, WorkspaceId,
};

const COLS: &str = "slack_channel_id, workspace_id, channel_id, thread_id, member_id, created_at";

pub async fn link(
    pool: &SqlitePool,
    new: NewSlackChannelLink,
) -> Result<SlackChannelLink, StoreError> {
    let now = Utc::now().to_rfc3339();
    let row = sqlx::query(&format!(
        "INSERT INTO maidan_slack_channel_links
           (slack_channel_id, workspace_id, channel_id, thread_id, member_id, created_at)
         VALUES (?, ?, ?, ?, ?, ?)
         ON CONFLICT (slack_channel_id) DO UPDATE
           SET workspace_id = excluded.workspace_id, channel_id = excluded.channel_id,
               thread_id = excluded.thread_id, member_id = excluded.member_id
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

pub async fn get(
    pool: &SqlitePool,
    slack_channel_id: &str,
) -> Result<Option<SlackChannelLink>, StoreError> {
    let row = sqlx::query(&format!(
        "SELECT {COLS} FROM maidan_slack_channel_links WHERE slack_channel_id = ?"
    ))
    .bind(slack_channel_id)
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

pub async fn unlink(pool: &SqlitePool, slack_channel_id: &str) -> Result<bool, StoreError> {
    let res = sqlx::query("DELETE FROM maidan_slack_channel_links WHERE slack_channel_id = ?")
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
    }
}

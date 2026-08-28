//! Slack projector channel links (Cluster 308): map a Slack channel to the Maidan
//! channel/thread it projects into. See the SQLite twin.

use sqlx::{PgPool, Row};

use crate::StoreError;
use maidan_types::{
    ChannelId, MemberId, NewSlackChannelLink, SlackChannelLink, ThreadId, WorkspaceId,
};

const COLS: &str = "slack_channel_id, workspace_id, channel_id, thread_id, member_id, created_at";

/// Create or replace the link for a Slack channel (one link per Slack channel).
pub async fn link(pool: &PgPool, new: NewSlackChannelLink) -> Result<SlackChannelLink, StoreError> {
    let row = sqlx::query(&format!(
        "INSERT INTO maidan_slack_channel_links
           (slack_channel_id, workspace_id, channel_id, thread_id, member_id, created_at)
         VALUES ($1, $2, $3, $4, $5, now())
         ON CONFLICT (slack_channel_id) DO UPDATE
           SET workspace_id = EXCLUDED.workspace_id, channel_id = EXCLUDED.channel_id,
               thread_id = EXCLUDED.thread_id, member_id = EXCLUDED.member_id
         RETURNING {COLS}"
    ))
    .bind(&new.slack_channel_id)
    .bind(new.workspace_id.0)
    .bind(new.channel_id.0)
    .bind(new.thread_id.0)
    .bind(new.member_id.0)
    .fetch_one(pool)
    .await?;
    Ok(row_to_link(&row))
}

pub async fn get(
    pool: &PgPool,
    slack_channel_id: &str,
) -> Result<Option<SlackChannelLink>, StoreError> {
    let row = sqlx::query(&format!(
        "SELECT {COLS} FROM maidan_slack_channel_links WHERE slack_channel_id = $1"
    ))
    .bind(slack_channel_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(row_to_link))
}

/// Resolve the link for a Maidan thread (the egress reverse lookup, Cluster 309).
/// One Slack channel per thread, so `LIMIT 1`.
pub async fn get_by_thread(
    pool: &PgPool,
    thread_id: ThreadId,
) -> Result<Option<SlackChannelLink>, StoreError> {
    let row = sqlx::query(&format!(
        "SELECT {COLS} FROM maidan_slack_channel_links WHERE thread_id = $1 LIMIT 1"
    ))
    .bind(thread_id.0)
    .fetch_optional(pool)
    .await?;
    Ok(row.as_ref().map(row_to_link))
}

pub async fn list(
    pool: &PgPool,
    workspace_id: WorkspaceId,
) -> Result<Vec<SlackChannelLink>, StoreError> {
    let rows = sqlx::query(&format!(
        "SELECT {COLS} FROM maidan_slack_channel_links WHERE workspace_id = $1 ORDER BY created_at DESC"
    ))
    .bind(workspace_id.0)
    .fetch_all(pool)
    .await?;
    Ok(rows.iter().map(row_to_link).collect())
}

pub async fn unlink(pool: &PgPool, slack_channel_id: &str) -> Result<bool, StoreError> {
    let res = sqlx::query("DELETE FROM maidan_slack_channel_links WHERE slack_channel_id = $1")
        .bind(slack_channel_id)
        .execute(pool)
        .await?;
    Ok(res.rows_affected() > 0)
}

fn row_to_link(row: &sqlx::postgres::PgRow) -> SlackChannelLink {
    SlackChannelLink {
        slack_channel_id: row.get("slack_channel_id"),
        workspace_id: WorkspaceId(row.get("workspace_id")),
        channel_id: ChannelId(row.get("channel_id")),
        thread_id: ThreadId(row.get("thread_id")),
        member_id: MemberId(row.get("member_id")),
        created_at: row.get("created_at"),
    }
}

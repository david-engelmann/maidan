//! Slack projector channel links: map a Slack channel to the Maidan
//! channel/thread it projects into. See the SQLite twin.

use sqlx::{PgPool, Row};

use crate::StoreError;
use maidan_types::{
    ChannelId, MemberId, NewSlackChannelLink, SlackChannelLink, ThreadId, WorkspaceId,
};

const COLS: &str =
    "slack_channel_id, workspace_id, channel_id, thread_id, member_id, created_at, disabled_at";

/// Create or replace the link for a Slack channel (one link per Slack channel).
pub async fn link(pool: &PgPool, new: NewSlackChannelLink) -> Result<SlackChannelLink, StoreError> {
    let row = sqlx::query(&format!(
        "INSERT INTO maidan_slack_channel_links
           (slack_channel_id, workspace_id, channel_id, thread_id, member_id, created_at)
         VALUES ($1, $2, $3, $4, $5, now())
         ON CONFLICT (workspace_id, slack_channel_id) DO UPDATE
           SET channel_id = EXCLUDED.channel_id, thread_id = EXCLUDED.thread_id,
               member_id = EXCLUDED.member_id, disabled_at = NULL
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

/// The workspace's own link for a Slack channel.
pub async fn get(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    slack_channel_id: &str,
) -> Result<Option<SlackChannelLink>, StoreError> {
    let row = sqlx::query(&format!(
        "SELECT {COLS} FROM maidan_slack_channel_links
         WHERE workspace_id = $1 AND slack_channel_id = $2"
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
    pool: &PgPool,
    scope: maidan_types::InboundScope,
    slack_channel_id: &str,
) -> Result<Vec<SlackChannelLink>, StoreError> {
    let sql = format!(
        "SELECT {COLS} FROM maidan_slack_channel_links
         WHERE slack_channel_id = $1 AND {} ORDER BY created_at",
        crate::inbound_scope::clause(scope, "workspace_id", "$2")
    );
    let query = sqlx::query(&sql).bind(slack_channel_id);
    let query = match crate::inbound_scope::bind(scope) {
        crate::inbound_scope::Bind::Workspace(id) => query.bind(id),
        crate::inbound_scope::Bind::Secret(name) => query.bind(name),
    };
    let rows = query.fetch_all(pool).await?;
    Ok(rows.iter().map(row_to_link).collect())
}

/// Resolve the link for a Maidan thread (the egress reverse lookup). One Slack
/// channel per thread, so `LIMIT 1`.
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

pub async fn unlink(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    slack_channel_id: &str,
) -> Result<bool, StoreError> {
    let res = sqlx::query(
        "DELETE FROM maidan_slack_channel_links
         WHERE workspace_id = $1 AND slack_channel_id = $2",
    )
    .bind(workspace_id.0)
    .bind(slack_channel_id)
    .execute(pool)
    .await?;
    Ok(res.rows_affected() > 0)
}

/// Turn egress to this Slack channel off. Idempotent: an already-disabled link
/// keeps its original timestamp, so a second failure does not reset when the
/// link broke. Returns whether this call did the disabling.
pub async fn disable(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    slack_channel_id: &str,
) -> Result<bool, StoreError> {
    let res = sqlx::query(
        "UPDATE maidan_slack_channel_links SET disabled_at = now()
         WHERE workspace_id = $1 AND slack_channel_id = $2 AND disabled_at IS NULL",
    )
    .bind(workspace_id.0)
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
        disabled_at: row.get("disabled_at"),
    }
}

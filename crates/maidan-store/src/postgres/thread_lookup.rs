//! Find a thread from where it is referenced outside Maidan.
//!
//! A pull request matches, in this order: the tracked change pull request
//! (the change flow's or a registered session's), a GitHub issue link, then
//! the newest result delivery commenting on it. A Slack message `ts` matches
//! the newest result delivery that replied in that Slack thread or was posted
//! as that message. A Slack channel alone matches the thread it is linked to.
//! Every branch is held to `workspace_id`.

use sqlx::{PgPool, Row};

use crate::StoreError;
use maidan_types::{ExternalThreadRef, ThreadId, WorkspaceId};

pub async fn find(
    pool: &PgPool,
    workspace_id: WorkspaceId,
    reference: &ExternalThreadRef,
) -> Result<Option<ThreadId>, StoreError> {
    let row = match reference {
        ExternalThreadRef::Pull { repo, number } => {
            sqlx::query(
                "SELECT thread_id FROM (
                   SELECT thread_id, 0 AS rank, opened_at AS at FROM maidan_change_pulls
                    WHERE workspace_id = $1 AND repo = $2 AND pull_number = $3
                   UNION ALL
                   SELECT thread_id, 1, created_at FROM maidan_github_issue_links
                    WHERE workspace_id = $1 AND lower(repo) = $2 AND issue_number = $3
                   UNION ALL
                   SELECT d.thread_id, 2, d.updated_at FROM maidan_result_deliveries d
                     JOIN maidan_threads t ON t.id = d.thread_id
                     JOIN maidan_channels c ON c.id = t.channel_id
                    WHERE c.workspace_id = $1 AND d.surface = 'github' AND lower(d.selector) = $4
                 ) found ORDER BY rank, at DESC LIMIT 1",
            )
            .bind(workspace_id.0)
            .bind(repo)
            .bind(number)
            .bind(format!("{repo}#{number}"))
            .fetch_optional(pool)
            .await?
        }
        ExternalThreadRef::Slack {
            channel,
            ts: Some(ts),
        } => {
            sqlx::query(
                "SELECT d.thread_id FROM maidan_result_deliveries d
                   JOIN maidan_threads t ON t.id = d.thread_id
                   JOIN maidan_channels c ON c.id = t.channel_id
                  WHERE c.workspace_id = $1 AND d.surface = 'slack'
                    AND (d.selector = $2
                         OR ((d.selector = $3 OR left(d.selector, length($4)) = $4)
                             AND d.external_ref = $5))
                  ORDER BY d.updated_at DESC LIMIT 1",
            )
            .bind(workspace_id.0)
            .bind(format!("{channel}/{ts}"))
            .bind(channel)
            .bind(format!("{channel}/"))
            .bind(ts)
            .fetch_optional(pool)
            .await?
        }
        ExternalThreadRef::Slack { channel, ts: None } => {
            sqlx::query(
                "SELECT thread_id FROM maidan_slack_channel_links
                  WHERE workspace_id = $1 AND slack_channel_id = $2",
            )
            .bind(workspace_id.0)
            .bind(channel)
            .fetch_optional(pool)
            .await?
        }
    };
    row.map(|row| row.try_get("thread_id").map(ThreadId))
        .transpose()
        .map_err(Into::into)
}

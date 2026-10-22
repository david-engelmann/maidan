//! Find a thread from where it is referenced outside Maidan. SQLite twin of
//! the Postgres module, which says what each reference matches.

use sqlx::{Row, SqlitePool};

use crate::StoreError;
use maidan_types::{ExternalThreadRef, ThreadId, WorkspaceId};

pub async fn find(
    pool: &SqlitePool,
    workspace_id: WorkspaceId,
    reference: &ExternalThreadRef,
) -> Result<Option<ThreadId>, StoreError> {
    let row = match reference {
        ExternalThreadRef::Pull { repo, number } => {
            sqlx::query(
                "SELECT thread_id FROM (
                   SELECT thread_id, 0 AS rank, opened_at AS at FROM maidan_change_pulls
                    WHERE workspace_id = ? AND repo = ? AND pull_number = ?
                   UNION ALL
                   SELECT thread_id, 1, created_at FROM maidan_github_issue_links
                    WHERE workspace_id = ? AND lower(repo) = ? AND issue_number = ?
                   UNION ALL
                   SELECT d.thread_id, 2, d.updated_at FROM maidan_result_deliveries d
                     JOIN maidan_threads t ON t.id = d.thread_id
                     JOIN maidan_channels c ON c.id = t.channel_id
                    WHERE c.workspace_id = ? AND d.surface = 'github' AND lower(d.selector) = ?
                 ) ORDER BY rank, at DESC LIMIT 1",
            )
            .bind(workspace_id.0)
            .bind(repo)
            .bind(number)
            .bind(workspace_id.0)
            .bind(repo)
            .bind(number)
            .bind(workspace_id.0)
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
                  WHERE c.workspace_id = ? AND d.surface = 'slack'
                    AND (d.selector = ?
                         OR ((d.selector = ? OR substr(d.selector, 1, length(?)) = ?)
                             AND d.external_ref = ?))
                  ORDER BY d.updated_at DESC LIMIT 1",
            )
            .bind(workspace_id.0)
            .bind(format!("{channel}/{ts}"))
            .bind(channel)
            .bind(format!("{channel}/"))
            .bind(format!("{channel}/"))
            .bind(ts)
            .fetch_optional(pool)
            .await?
        }
        ExternalThreadRef::Slack { channel, ts: None } => {
            sqlx::query(
                "SELECT thread_id FROM maidan_slack_channel_links
                  WHERE workspace_id = ? AND slack_channel_id = ?",
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

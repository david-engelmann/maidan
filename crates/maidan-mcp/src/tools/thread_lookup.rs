//! `find_thread`: the thread a Slack message or channel, or a GitHub pull
//! request, refers to. The twin of `GET /workspaces/{id}/thread-lookup`, by
//! the same [`maidan_auth::find_readable_thread`].

use maidan_auth::AuthContext;
use maidan_types::{ThreadLookup, WorkspaceId};
use serde::Deserialize;
use serde_json::Value;

use super::content_json;
use crate::error::McpError;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FindThreadArgs {
    /// Omitted, the caller's own workspace; never every workspace.
    #[serde(default)]
    workspace_id: Option<uuid::Uuid>,
    #[serde(default)]
    slack_channel: Option<String>,
    #[serde(default)]
    slack_ts: Option<String>,
    #[serde(default)]
    repo: Option<String>,
    #[serde(default)]
    pr_number: Option<i64>,
}

/// The thread, or `null` when nothing the caller may read matches.
pub(super) async fn find_thread(
    server: &crate::server::McpServer,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: FindThreadArgs = super::parse_args(args)?;
    let workspace_id = a.workspace_id.map_or(auth.workspace_id, WorkspaceId);
    auth.ensure_workspace(workspace_id)?;
    let lookup = ThreadLookup {
        slack_channel: a.slack_channel,
        slack_ts: a.slack_ts,
        repo: a.repo,
        pr_number: a.pr_number,
    };
    let thread =
        maidan_auth::find_readable_thread(server.store.as_ref(), auth, workspace_id, &lookup)
            .await?;
    Ok(content_json(&thread))
}

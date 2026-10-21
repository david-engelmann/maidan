//! A session's pull request and the swarm metrics over MCP: the twins of
//! `PUT /threads/{id}/pull` and `GET /workspaces/{id}/swarm-metrics`, so a
//! session registers its pull request and reads the ramp numbers without
//! leaving its MCP connection. Both run the same
//! [`maidan_store::change_pulls`] code as the routes.

use maidan_auth::AuthContext;
use maidan_store::change_pulls::{self, ChangePullError, SwarmMetricsFilter};
use maidan_types::{RegisterPull, TaskEngine, TaskKind, TaskSize, ThreadId, WorkspaceId};
use serde::Deserialize;
use serde_json::Value;

use super::content_json;
use crate::error::McpError;

impl From<ChangePullError> for McpError {
    fn from(err: ChangePullError) -> Self {
        match err {
            ChangePullError::Invalid(msg) => Self::InvalidParams(msg),
            ChangePullError::NotAllowed(msg) => Self::Forbidden(msg),
            ChangePullError::Store(err) => err.into(),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RegisterPullArgs {
    thread_id: uuid::Uuid,
    repo: String,
    pull_number: i64,
    #[serde(default)]
    claimed_at: Option<chrono::DateTime<chrono::Utc>>,
}

/// Track a session's pull request on its task. `workspace:write` (checked
/// before dispatch) and thread access, checked here as the route checks it,
/// since the thread names the workspace whose allowlist decides.
pub(super) async fn register_thread_pull(
    server: &crate::server::McpServer,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: RegisterPullArgs = super::parse_args(args)?;
    let thread_id = ThreadId(a.thread_id);
    let scope = maidan_auth::authorize_thread(server.store.as_ref(), auth, thread_id).await?;
    let registered = change_pulls::register_session_pull(
        server.store.as_ref(),
        scope.workspace_id,
        thread_id,
        &RegisterPull {
            repo: a.repo,
            pull_number: a.pull_number,
            claimed_at: a.claimed_at,
        },
    )
    .await?;
    Ok(content_json(&registered))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SwarmMetricsArgs {
    workspace_id: uuid::Uuid,
    #[serde(default)]
    repo: Option<String>,
    #[serde(default)]
    since: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default)]
    rounds_limit: Option<i64>,
    #[serde(default)]
    behind_limit: Option<i64>,
    #[serde(default)]
    engine: Option<TaskEngine>,
    #[serde(default)]
    kind: Option<TaskKind>,
    #[serde(default)]
    size: Option<TaskSize>,
    #[serde(default)]
    linear_issue: Option<String>,
}

/// The workspace's ramp metrics, after the same GitHub refresh the route
/// runs when the server has a GitHub client. `workspace:read`, and the
/// workspace must be the caller's.
pub(super) async fn get_swarm_metrics(
    server: &crate::server::McpServer,
    auth: &AuthContext,
    args: &Value,
) -> Result<Value, McpError> {
    let a: SwarmMetricsArgs = super::parse_args(args)?;
    let workspace_id = WorkspaceId(a.workspace_id);
    // The pre-dispatch gate passes an id it cannot parse; this one parsed.
    auth.ensure_workspace(workspace_id)?;
    let query = SwarmMetricsFilter {
        repo: a.repo,
        since: a.since,
        rounds_limit: a.rounds_limit,
        behind_limit: a.behind_limit,
        engine: a.engine,
        kind: a.kind,
        size: a.size,
        linear_issue: a.linear_issue,
    }
    .check()?;
    if let Some(refresher) = server.pull_refresher() {
        refresher.refresh_open_pulls(workspace_id).await;
    }
    let metrics = change_pulls::swarm_metrics(server.store.as_ref(), workspace_id, &query).await?;
    Ok(content_json(&metrics))
}

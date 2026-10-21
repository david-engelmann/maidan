//! Reading tracked pull requests' state from GitHub before the swarm metrics
//! are computed. The HTTP server holds the GitHub client, so it installs
//! this ([`crate::McpServer::set_pull_refresher`]); without it, as in tests
//! and on an instance with no GitHub token, `get_swarm_metrics` reports what
//! the store holds, as the route does.

use maidan_types::WorkspaceId;

#[async_trait::async_trait]
pub trait OpenPullRefresher: Send + Sync {
    /// Best effort: a failed read leaves a pull request as the store has it.
    async fn refresh_open_pulls(&self, workspace_id: WorkspaceId);
}

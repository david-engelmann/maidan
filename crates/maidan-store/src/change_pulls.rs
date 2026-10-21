//! Registering a session's pull request on its task and reading the swarm
//! metrics, shared by `PUT /threads/{id}/pull`, `GET
//! /workspaces/{id}/swarm-metrics` and their MCP twins, so the two surfaces
//! apply one allowlist rule and one set of filters. Capability and thread
//! access stay with each caller, which knows how its credential arrived.

use chrono::{DateTime, Utc};
use maidan_types::{
    FlagLimits, RegisterPull, RegisteredPull, SwarmMetrics, TaskEngine, TaskKind, TaskSize,
    ThreadId, WorkspaceId,
};

use crate::error::StoreError;
use crate::store::Store;

/// Why a registration or a metrics read was refused.
#[derive(Debug, thiserror::Error)]
pub enum ChangePullError {
    /// The request is malformed: a 400, or invalid params over MCP.
    #[error("{0}")]
    Invalid(String),
    /// The repository is not blessed for the workspace: a 403.
    #[error("{0}")]
    NotAllowed(String),
    #[error(transparent)]
    Store(#[from] StoreError),
}

/// Track a pull request the change flow did not open on `thread_id`. The
/// repository must be blessed for the workspace (GitHub comments, or agent
/// branches), so a tenant cannot read another's pull requests through the
/// change sweep. The caller has already checked the thread is in
/// `workspace_id` and that it may write there.
pub async fn register_session_pull(
    store: &dyn Store,
    workspace_id: WorkspaceId,
    thread_id: ThreadId,
    request: &RegisterPull,
) -> Result<RegisteredPull, ChangePullError> {
    let repo = maidan_types::normalize_repo(&request.repo).map_err(ChangePullError::Invalid)?;
    if request.pull_number < 1 {
        return Err(ChangePullError::Invalid("pull_number is at least 1".into()));
    }
    let blessed = crate::pull_watch::repo_blessed(store, workspace_id, &repo).await?;
    if !blessed {
        return Err(ChangePullError::NotAllowed(format!(
            "`{repo}` is not in the workspace egress allowlist"
        )));
    }
    let opened_at = store
        .register_change_pull(
            workspace_id,
            thread_id,
            &repo,
            request.pull_number,
            request.claimed_at,
        )
        .await?;
    Ok(RegisteredPull {
        repo,
        pull_number: request.pull_number,
        opened_at,
    })
}

/// The swarm-metrics filters as a caller gives them.
#[derive(Debug, Clone, Default)]
pub struct SwarmMetricsFilter {
    pub repo: Option<String>,
    pub since: Option<DateTime<Utc>>,
    pub rounds_limit: Option<i64>,
    pub behind_limit: Option<i64>,
    pub engine: Option<TaskEngine>,
    pub kind: Option<TaskKind>,
    pub size: Option<TaskSize>,
    /// Comma-separated Linear issue keys.
    pub linear_issue: Option<String>,
    /// The provider, or the harness, that spent the most on each task, as
    /// its usage reports named it.
    pub provider: Option<String>,
    pub harness: Option<String>,
}

/// [`SwarmMetricsFilter`] checked, ready for [`swarm_metrics`]. Checked
/// before anything is read from GitHub, so a bad filter costs no API calls.
#[derive(Debug, Clone)]
pub struct SwarmMetricsQuery {
    repo: Option<String>,
    since: Option<DateTime<Utc>>,
    limits: FlagLimits,
    engine: Option<TaskEngine>,
    kind: Option<TaskKind>,
    size: Option<TaskSize>,
    issues: Option<Vec<String>>,
    provider: Option<String>,
    harness: Option<String>,
}

impl SwarmMetricsFilter {
    pub fn check(self) -> Result<SwarmMetricsQuery, ChangePullError> {
        let repo = self
            .repo
            .map(|repo| maidan_types::normalize_repo(&repo).map_err(ChangePullError::Invalid))
            .transpose()?;
        let defaults = FlagLimits::default();
        let limits = FlagLimits {
            rounds: self.rounds_limit.unwrap_or(defaults.rounds).max(1),
            behind: self.behind_limit.unwrap_or(defaults.behind).max(1),
        };
        let issues: Option<Vec<String>> = self.linear_issue.as_deref().map(|list| {
            list.split(',')
                .map(str::trim)
                .filter(|key| !key.is_empty())
                .map(str::to_string)
                .collect()
        });
        if let Some(bad) = issues
            .iter()
            .flatten()
            .find(|key| !maidan_types::is_issue_key(key))
        {
            return Err(ChangePullError::Invalid(format!(
                "linear_issue `{bad}` is not an issue key such as ENG-123"
            )));
        }
        Ok(SwarmMetricsQuery {
            repo,
            since: self.since,
            limits,
            engine: self.engine,
            kind: self.kind,
            size: self.size,
            issues,
            provider: self.provider,
            harness: self.harness,
        })
    }
}

/// The workspace's ramp metrics as the store holds them now. A caller that
/// can read GitHub refreshes the open pull requests first, so a merge counts
/// without waiting for the next sweep.
pub async fn swarm_metrics(
    store: &dyn Store,
    workspace_id: WorkspaceId,
    query: &SwarmMetricsQuery,
) -> Result<SwarmMetrics, StoreError> {
    let mut pulls = store
        .list_change_pulls(
            workspace_id,
            query.repo.as_deref(),
            query.since,
            query.engine,
            query.kind,
            query.size,
        )
        .await?;
    if let Some(provider) = query.provider.as_deref() {
        pulls.retain(|pull| pull.provider.as_deref() == Some(provider));
    }
    if let Some(harness) = query.harness.as_deref() {
        pulls.retain(|pull| pull.harness.as_deref() == Some(harness));
    }
    if let Some(issues) = &query.issues {
        pulls.retain(|pull| {
            pull.attributes
                .linear_issue
                .as_ref()
                .or(pull.linear_issue.as_ref())
                .is_some_and(|key| issues.contains(key))
        });
    }
    Ok(maidan_types::swarm_metrics(
        pulls,
        query.limits,
        chrono::Utc::now(),
    ))
}

//! Swarm ramp metrics over REST (see [`maidan_types::change_metrics`]): per
//! agent pull request, the rounds it took, how long to ready and to merge,
//! whether it merged as delivered, what it cost, and whether it needs a look.

use axum::{extract::State, Extension, Json};
use maidan_auth::capability::WORKSPACE_READ;
use maidan_auth::AuthContext;
use maidan_types::{FlagLimits, SwarmMetrics, WorkspaceId};

use crate::dto::SwarmMetricsQuery;
use crate::error::ApiError;
use crate::extract::{ApiJson, ApiPath, ApiQuery};
use crate::routes::{cap, ensure_workspace, ApiResult};
use crate::state::AppState;

/// `PUT /threads/:id/pull` — track a pull request the change flow did not
/// open (a session's) on this task, so the change sweep refreshes it and the
/// swarm metrics count it: its time to ready runs from the task's `CLAIM:`
/// comment (else the `claimed_at` given, for a claim said outside Maidan,
/// else now) to the `ready-to-merge` label, and its merge closes
/// the task. `workspace:write` and thread access. The repository must be
/// blessed for the workspace (GitHub comments, or agent branches), so a
/// tenant cannot read another's pull requests through the sweep.
pub async fn register_thread_pull(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
    ApiJson(body): ApiJson<maidan_types::RegisterPull>,
) -> ApiResult<Json<maidan_types::RegisteredPull>> {
    cap(&auth, maidan_auth::capability::WORKSPACE_WRITE)?;
    let thread_id = maidan_types::ThreadId(id);
    let ctx = maidan_auth::authorize_thread(state.store.as_ref(), &auth, thread_id).await?;
    let repo = maidan_types::normalize_repo(&body.repo).map_err(ApiError::BadRequest)?;
    if body.pull_number < 1 {
        return Err(ApiError::BadRequest("pull_number is at least 1".into()));
    }
    let blessed = state
        .store
        .list_egress_targets(ctx.workspace_id)
        .await?
        .into_iter()
        .any(|target| match target.surface.as_str() {
            "github" => target.selector.eq_ignore_ascii_case(&repo),
            "github_branch" => target
                .selector
                .split_once('@')
                .is_some_and(|(r, _)| r.eq_ignore_ascii_case(&repo)),
            _ => false,
        });
    if !blessed {
        return Err(ApiError::Forbidden(format!(
            "`{repo}` is not in the workspace egress allowlist"
        )));
    }
    let opened_at = state
        .store
        .register_change_pull(
            ctx.workspace_id,
            thread_id,
            &repo,
            body.pull_number,
            body.claimed_at,
        )
        .await?;
    Ok(Json(maidan_types::RegisteredPull {
        repo,
        pull_number: body.pull_number,
        opened_at,
    }))
}

/// The workspace's ramp metrics. Before computing them, pull requests not yet
/// seen closed are read from GitHub (the change sweep's refresh), so a merge
/// counts without waiting for the next sweep. That refresh fills in what the
/// read reports (a merge time GitHub already holds), with no event or audit
/// row, so the route is classified a read; closing the merged change's thread
/// is left to the sweep. `workspace:read`.
pub async fn get_swarm_metrics(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
    ApiQuery(query): ApiQuery<SwarmMetricsQuery>,
) -> ApiResult<Json<SwarmMetrics>> {
    cap(&auth, WORKSPACE_READ)?;
    let workspace_id = WorkspaceId(id);
    ensure_workspace(&auth, workspace_id)?;
    let repo = query
        .repo
        .map(|repo| maidan_types::normalize_repo(&repo).map_err(ApiError::BadRequest))
        .transpose()?;
    let defaults = FlagLimits::default();
    let limits = FlagLimits {
        rounds: query.rounds_limit.unwrap_or(defaults.rounds).max(1),
        behind: query.behind_limit.unwrap_or(defaults.behind).max(1),
    };
    let issues: Option<Vec<String>> = query.linear_issue.as_deref().map(|list| {
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
        return Err(ApiError::BadRequest(format!(
            "linear_issue `{bad}` is not an issue key such as ENG-123"
        )));
    }
    crate::change_sweep::refresh_open_pulls(&state, workspace_id).await;
    let mut pulls = state
        .store
        .list_change_pulls(
            workspace_id,
            repo.as_deref(),
            query.since,
            query.engine,
            query.kind,
            query.size,
        )
        .await?;
    if let Some(issues) = issues {
        pulls.retain(|pull| {
            pull.attributes
                .linear_issue
                .as_ref()
                .or(pull.linear_issue.as_ref())
                .is_some_and(|key| issues.contains(key))
        });
    }
    Ok(Json(maidan_types::swarm_metrics(
        pulls,
        limits,
        chrono::Utc::now(),
    )))
}

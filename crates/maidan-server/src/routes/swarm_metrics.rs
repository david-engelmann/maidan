//! Swarm ramp metrics over REST (see [`maidan_types::change_metrics`]): per
//! agent pull request, the rounds it took, how long to ready and to merge,
//! whether it merged as delivered, what it cost, and whether it needs a look.

use axum::{extract::State, Extension, Json};
use maidan_auth::capability::WORKSPACE_READ;
use maidan_auth::AuthContext;
use maidan_types::{FlagLimits, SwarmMetrics, WorkspaceId};

use crate::dto::SwarmMetricsQuery;
use crate::error::ApiError;
use crate::extract::{ApiPath, ApiQuery};
use crate::routes::{cap, ensure_workspace, ApiResult};
use crate::state::AppState;

/// Tracked pull requests refreshed per read: enough for a ramp, bounded so a
/// read never fans out into hundreds of GitHub calls.
const REFRESH_LIMIT: i64 = 25;

/// The workspace's ramp metrics. Before computing them, pull requests not yet
/// seen closed are read from GitHub, oldest first, so a merge counts without
/// a webhook (a local stack may receive none). That refresh fills in what
/// the read reports (a merge time GitHub already holds), with no event or
/// audit row, so the route is classified a read. `workspace:read`.
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
    refresh_open_pulls(&state, workspace_id).await;
    let pulls = state
        .store
        .list_change_pulls(workspace_id, repo.as_deref(), query.since)
        .await?;
    Ok(Json(maidan_types::swarm_metrics(pulls, limits)))
}

/// Record the close of each tracked pull request GitHub now reports closed.
/// Best effort: a failed read leaves it open until the next one.
async fn refresh_open_pulls(state: &AppState, workspace_id: WorkspaceId) {
    let Some(git) = state.github_sender.as_ref().and_then(|s| s.git()) else {
        return;
    };
    let open = match state
        .store
        .list_open_change_pulls(workspace_id, REFRESH_LIMIT)
        .await
    {
        Ok(open) => open,
        Err(err) => {
            tracing::warn!(error = %err, "swarm metrics: open pull requests not read");
            return;
        }
    };
    for (repo, pull_number) in open {
        let brief = match git.pull_brief(&repo, pull_number).await {
            Ok(brief) => brief,
            Err(err) => {
                tracing::debug!(repo, pull_number, error = %err, "swarm metrics: pull request not refreshed");
                continue;
            }
        };
        if brief.open {
            continue;
        }
        let closed_at = brief
            .closed_at
            .or(brief.merged_at)
            .unwrap_or_else(chrono::Utc::now);
        let merged_head = brief.merged_at.map(|_| brief.head_sha.as_str());
        if let Err(err) = state
            .store
            .close_change_pull(&repo, pull_number, brief.merged_at, merged_head, closed_at)
            .await
        {
            tracing::warn!(repo, pull_number, error = %err, "swarm metrics: close not recorded");
        }
    }
}

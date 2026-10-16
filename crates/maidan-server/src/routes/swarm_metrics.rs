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
    crate::change_sweep::refresh_open_pulls(&state, workspace_id).await;
    let pulls = state
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
    Ok(Json(maidan_types::swarm_metrics(pulls, limits)))
}

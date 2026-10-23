//! A month of usage against the instance's free allowance: a workspace's own
//! (`GET /workspaces/:id/usage-meters`, also behind the console's session) and every
//! workspace's for an operator (`GET /operator/usage-meters`). Read-only: nothing is
//! cut off or billed.

use axum::{extract::State, Extension, Json};
use maidan_auth::capability::{OPERATOR_GLOBAL, WORKSPACE_READ};
use maidan_auth::AuthContext;
use maidan_types::{MonthlyUsage, UsageMonth, WorkspaceId};

use crate::dto::UsageMonthQuery;
use crate::error::ApiError;
use crate::extract::{ApiPath, ApiQuery};
use crate::routes::{cap, ensure_workspace, ApiResult};
use crate::state::AppState;

/// The month asked for, or the current UTC month.
fn month(query: &UsageMonthQuery) -> ApiResult<UsageMonth> {
    match query.month.as_deref() {
        None => Ok(UsageMonth::of(chrono::Utc::now().date_naive())),
        Some(raw) => UsageMonth::parse(raw)
            .ok_or_else(|| ApiError::BadRequest(format!("month `{raw}` is not YYYY-MM"))),
    }
}

/// `GET /workspaces/:id/usage-meters?month=YYYY-MM`: the caller's workspace's month,
/// every meter against the allowance. `workspace:read`.
pub async fn get_workspace_usage_meters(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
    ApiQuery(query): ApiQuery<UsageMonthQuery>,
) -> ApiResult<Json<MonthlyUsage>> {
    cap(&auth, WORKSPACE_READ)?;
    let workspace_id = WorkspaceId(id);
    ensure_workspace(&auth, workspace_id)?;
    let month = month(&query)?;
    let rows = state
        .store
        .list_usage_meters(workspace_id, month.first)
        .await?;
    Ok(Json(month.usage(
        workspace_id,
        &rows,
        &state.usage_allowance,
    )))
}

/// `GET /operator/usage-meters?month=YYYY-MM`: every workspace that used anything in
/// the month, each meter against the allowance. `operator:global`.
pub async fn list_operator_usage_meters(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiQuery(query): ApiQuery<UsageMonthQuery>,
) -> ApiResult<Json<Vec<MonthlyUsage>>> {
    cap(&auth, OPERATOR_GLOBAL)?;
    let month = month(&query)?;
    let rows = state
        .store
        .list_usage_meters_between(month.first, month.next)
        .await?;
    let mut by_workspace: std::collections::BTreeMap<uuid::Uuid, Vec<maidan_types::UsageMeterDay>> =
        std::collections::BTreeMap::new();
    for row in rows {
        by_workspace
            .entry(row.workspace_id.0)
            .or_default()
            .push(row);
    }
    Ok(Json(
        by_workspace
            .into_iter()
            .map(|(id, rows)| month.usage(WorkspaceId(id), &rows, &state.usage_allowance))
            .collect(),
    ))
}

//! Usage rollups and OpenTelemetry-shaped usage ingest.

use axum::{extract::State, Extension, Json};
use maidan_auth::{capability::THREAD_TRANSITION, AuthContext};
use maidan_types::*;

use super::{cap, ensure_workspace, publish_stored, ApiResult};
use crate::error::ApiError;
use crate::extract::{ApiJson, ApiPath, ApiQuery};
use crate::state::AppState;

async fn rollup(state: &AppState, query: UsageRollupQuery) -> ApiResult<Json<UsageRollup>> {
    Ok(Json(state.store.usage_rollup(query).await?))
}

/// Spend, hit rate, write share, dollars saved, and cost per completed task
/// for one thread. `workspace:read` plus thread access.
pub async fn get_thread_usage_rollup(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
) -> ApiResult<Json<UsageRollup>> {
    cap(&auth, maidan_auth::capability::WORKSPACE_READ)?;
    let thread_id = ThreadId(id);
    maidan_auth::ensure_thread_access(state.store.as_ref(), &auth, thread_id).await?;
    let thread = state.store.get_thread(thread_id).await?;
    let channel = state.store.get_channel(thread.channel_id).await?;
    rollup(
        &state,
        UsageRollupQuery {
            workspace_id: channel.workspace_id,
            thread_id: Some(thread_id),
            member_id: None,
        },
    )
    .await
}

/// The same rollup for every completed report in the workspace.
pub async fn get_workspace_usage_rollup(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
) -> ApiResult<Json<UsageRollup>> {
    let workspace_id = WorkspaceId(id);
    cap(&auth, maidan_auth::capability::WORKSPACE_READ)?;
    ensure_workspace(&auth, workspace_id)?;
    state.store.get_workspace(workspace_id).await?;
    rollup(
        &state,
        UsageRollupQuery {
            workspace_id,
            thread_id: None,
            member_id: None,
        },
    )
    .await
}

/// Fleet conformance: the workspace's newest completed usage reports read
/// against a published harness profile (harness, version, provider, model,
/// boot pack), per member. `workspace:read`, the caller's workspace only.
pub async fn get_workspace_usage_conformance(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
    ApiQuery(query): ApiQuery<crate::dto::FleetConformanceQuery>,
) -> ApiResult<Json<FleetConformance>> {
    let workspace_id = WorkspaceId(id);
    cap(&auth, maidan_auth::capability::WORKSPACE_READ)?;
    ensure_workspace(&auth, workspace_id)?;
    state.store.get_workspace(workspace_id).await?;
    let limit = query.limit.unwrap_or(200);
    if !(1..=1000).contains(&limit) {
        return Err(ApiError::BadRequest("limit must be 1 to 1000".into()));
    }
    let profile = FleetProfile {
        harness: query.harness,
        harness_version: query.harness_version,
        provider: query.provider,
        model: query.model,
        boot_pack_sha256: query.boot_pack_sha256,
    };
    let entries = state
        .store
        .list_workspace_usage_ledger(workspace_id, limit)
        .await?;
    Ok(Json(profile.check(workspace_id, &entries)))
}

/// Model and effort by task class (C12): per thread kind and size, each
/// model and effort's tasks, completed tasks and cost per completed task
/// from the ledger, and the cheapest combination with at least
/// `min_completed` completed tasks as the suggestion for the next task in
/// that class. `workspace:read`, the caller's workspace only.
pub async fn get_workspace_usage_routing(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
    ApiQuery(query): ApiQuery<crate::dto::UsageRoutingQuery>,
) -> ApiResult<Json<TaskClassRouting>> {
    let workspace_id = WorkspaceId(id);
    cap(&auth, maidan_auth::capability::WORKSPACE_READ)?;
    ensure_workspace(&auth, workspace_id)?;
    state.store.get_workspace(workspace_id).await?;
    let min_completed = query.min_completed.unwrap_or(3);
    if !(1..=1000).contains(&min_completed) {
        return Err(ApiError::BadRequest(
            "min_completed must be 1 to 1000".into(),
        ));
    }
    let rows = state.store.task_class_usage(workspace_id).await?;
    TaskClassRouting::from_rows(workspace_id, min_completed, &rows)
        .map(Json)
        .map_err(ApiError::BadRequest)
}

/// Per-seat cost (#1771): the workspace's completed usage reports accepted in
/// `[since, until)`, summed by the member that reported them, most expensive
/// first. Seats are open-ended, so this is a ledger read rather than a metric
/// label. `workspace:read`, the caller's workspace only.
pub async fn get_workspace_usage_seats(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
    ApiQuery(params): ApiQuery<crate::dto::SeatCostParams>,
) -> ApiResult<Json<SeatCosts>> {
    let workspace_id = WorkspaceId(id);
    cap(&auth, maidan_auth::capability::WORKSPACE_READ)?;
    ensure_workspace(&auth, workspace_id)?;
    state.store.get_workspace(workspace_id).await?;
    let query = SeatCostQuery {
        workspace_id,
        since: params.since,
        until: params.until,
        limit: params.limit.unwrap_or(100),
    };
    query.validate().map_err(ApiError::BadRequest)?;
    Ok(Json(state.store.seat_costs(query).await?))
}

/// The same rollup for one member of the caller's workspace.
pub async fn get_member_usage_rollup(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
) -> ApiResult<Json<UsageRollup>> {
    cap(&auth, maidan_auth::capability::WORKSPACE_READ)?;
    let member_id = MemberId(id);
    state
        .store
        .get_member_in(auth.workspace_id, member_id)
        .await?;
    rollup(
        &state,
        UsageRollupQuery {
            workspace_id: auth.workspace_id,
            thread_id: None,
            member_id: Some(member_id),
        },
    )
    .await
}

/// Accept one usage report whose token counts come from GenAI attributes.
/// The price snapshot is still the reporter's. `usd_micros` is computed.
pub async fn report_thread_usage_otel(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
    ApiJson(body): ApiJson<GenAiUsageReport>,
) -> ApiResult<Json<UsageLedgerEntry>> {
    cap(&auth, THREAD_TRANSITION)?;
    let thread_id = ThreadId(id);
    maidan_auth::ensure_thread_access(state.store.as_ref(), &auth, thread_id).await?;
    let reporter = if auth.bypass {
        state
            .store
            .get_thread(thread_id)
            .await?
            .assignee_id
            .ok_or_else(|| ApiError::Conflict("thread has no active claim holder".into()))?
    } else {
        auth.member_id
    };
    let new = body
        .into_new(thread_id, reporter)
        .map_err(ApiError::BadRequest)?;
    let (report, stored) = state.store.report_accounted_usage(&new).await?;
    for stored in stored {
        publish_stored(&state, stored).await;
    }
    Ok(Json(report))
}

//! Task-schedule management (Cluster 228): create / list / pause-resume / delete
//! the schedules that the sweeper (Cluster 227) fires. A schedule materializes a
//! task thread in its channel when due, so the write surfaces are gated on
//! `workspace:write` + access to the target channel.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use maidan_auth::{
    capability::{WORKSPACE_READ, WORKSPACE_WRITE},
    AuthContext,
};
use maidan_router::resolve_channel_context;
use maidan_types::*;

use super::{cap, ensure_workspace, ApiResult};
use crate::dto::*;
use crate::error::{ApiError, ApiJson};
use crate::state::AppState;

pub async fn create_task_schedule(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(workspace_id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<CreateTaskSchedule>,
) -> ApiResult<(StatusCode, Json<TaskSchedule>)> {
    let workspace_id = WorkspaceId(workspace_id);
    cap(&auth, WORKSPACE_WRITE)?;
    ensure_workspace(&auth, workspace_id)?;

    if body.title.trim().is_empty() {
        return Err(ApiError::BadRequest("title must not be empty".into()));
    }
    if let Some(secs) = body.interval_secs {
        if secs <= 0 {
            return Err(ApiError::BadRequest(
                "interval_secs must be positive (omit for a one-shot)".into(),
            ));
        }
    }

    let channel_id = ChannelId(body.channel_id);
    let ctx = resolve_channel_context(state.store.as_ref(), channel_id).await?;
    if ctx.workspace_id != workspace_id {
        return Err(ApiError::BadRequest(
            "channel is not in this workspace".into(),
        ));
    }
    maidan_auth::ensure_channel_access(state.store.as_ref(), &auth, channel_id).await?;

    let schedule = state
        .store
        .create_task_schedule(NewTaskSchedule {
            workspace_id,
            channel_id,
            title: body.title.trim().to_string(),
            interval_secs: body.interval_secs,
            next_run_at: body.first_run_at.unwrap_or_else(chrono::Utc::now),
            created_by: auth.member_id,
        })
        .await?;
    Ok((StatusCode::CREATED, Json(schedule)))
}

pub async fn list_task_schedules(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(workspace_id): Path<uuid::Uuid>,
) -> ApiResult<Json<Vec<TaskSchedule>>> {
    let workspace_id = WorkspaceId(workspace_id);
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, workspace_id)?;
    let schedules = state.store.list_task_schedules(workspace_id).await?;
    Ok(Json(schedules))
}

/// Resolve a schedule and authorize the caller for it: workspace membership plus
/// access to its target channel.
async fn authorize_schedule(
    state: &AppState,
    auth: &AuthContext,
    id: TaskScheduleId,
) -> ApiResult<TaskSchedule> {
    let schedule = state.store.get_task_schedule(id).await?;
    ensure_workspace(auth, schedule.workspace_id)?;
    maidan_auth::ensure_channel_access(state.store.as_ref(), auth, schedule.channel_id).await?;
    Ok(schedule)
}

pub async fn set_task_schedule_active(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<SetTaskScheduleActive>,
) -> ApiResult<Json<TaskSchedule>> {
    cap(&auth, WORKSPACE_WRITE)?;
    let id = TaskScheduleId(id);
    authorize_schedule(&state, &auth, id).await?;
    let updated = state
        .store
        .set_task_schedule_active(id, body.active)
        .await?;
    Ok(Json(updated))
}

pub async fn delete_task_schedule(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<StatusCode> {
    cap(&auth, WORKSPACE_WRITE)?;
    let id = TaskScheduleId(id);
    authorize_schedule(&state, &auth, id).await?;
    if state.store.delete_task_schedule(id).await? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

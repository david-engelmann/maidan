//! Planner fan-out over REST (#1421): the planner holding a plan parent's
//! claim submits its children in one call.

use axum::{extract::State, http::StatusCode, Extension, Json};
use maidan_auth::capability::WORKSPACE_WRITE;
use maidan_auth::AuthContext;
use maidan_types::{PlanChildren, PlanOutcome, SubmitPlan, ThreadId};

use crate::error::ApiError;
use crate::extract::{ApiJson, ApiPath};
use crate::routes::{cap, ApiResult};
use crate::state::AppState;

/// Make the plan's children: `201` with `{children: [{key, thread_id}]}`.
/// A repeat from the same claim with the same keys is a `200` with the
/// children the first call made; any other plan on a parent that has
/// children is a `409` listing them. A malformed plan is a `400` naming the
/// child, and nothing is created. `workspace:write` + thread access; the
/// store checks the caller holds the parent's live claim under `lease_id`.
pub async fn submit_plan(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
    ApiJson(body): ApiJson<SubmitPlan>,
) -> ApiResult<(StatusCode, Json<PlanChildren>)> {
    cap(&auth, WORKSPACE_WRITE)?;
    let parent = ThreadId(id);
    maidan_auth::authorize_thread(state.store.as_ref(), &auth, parent).await?;
    let plan = body.validated().map_err(ApiError::BadRequest)?;
    match state
        .store
        .submit_plan(parent, auth.member_id, plan)
        .await?
    {
        PlanOutcome::Created { children, events } => {
            for stored in events {
                super::publish_stored(&state, stored).await;
            }
            Ok((StatusCode::CREATED, Json(PlanChildren { children })))
        }
        PlanOutcome::Replayed(children) => Ok((StatusCode::OK, Json(PlanChildren { children }))),
        PlanOutcome::Conflict(children) => Err(ApiError::PlanConflict(children)),
    }
}

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
    let plan = body.checked().map_err(ApiError::BadRequest)?;
    let held = plan.needs_hold();
    match state
        .store
        .submit_plan(parent, auth.member_id, plan)
        .await?
    {
        PlanOutcome::Created { children, events } => {
            for stored in events {
                super::publish_stored(&state, stored).await;
            }
            Ok((StatusCode::CREATED, Json(PlanChildren { children, held })))
        }
        PlanOutcome::Replayed(children) => {
            Ok((StatusCode::OK, Json(PlanChildren { children, held })))
        }
        PlanOutcome::Conflict(children) => Err(ApiError::PlanConflict(children)),
    }
}

/// `POST /threads/:id/plan/amend` — change a submitted plan under the
/// planner's live claim: cancel unclaimed children, add children, rewire
/// unclaimed children's dependencies, all in one transaction. `200` with what
/// changed; a malformed amendment is a `400`; a claimed or finished child,
/// or a fourth amendment, is a `409`. `workspace:write` + thread access.
pub async fn amend_plan(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
    ApiJson(body): ApiJson<maidan_types::AmendPlan>,
) -> ApiResult<Json<maidan_types::PlanAmendment>> {
    cap(&auth, WORKSPACE_WRITE)?;
    let parent = ThreadId(id);
    maidan_auth::authorize_thread(state.store.as_ref(), &auth, parent).await?;
    let amend = body.checked().map_err(ApiError::BadRequest)?;
    let outcome = state
        .store
        .amend_plan(parent, auth.member_id, amend)
        .await?;
    for stored in outcome.events {
        super::publish_stored(&state, stored).await;
    }
    Ok(Json(outcome.amendment))
}

/// `POST /threads/:id/plan/release` — release a held plan: its children
/// become claimable. A plan with five or more children, or any child that
/// needs a threat model, is created held. `token:admin` (the Maintainer),
/// audited; releasing a plan that holds nothing answers an empty list.
pub async fn release_plan(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
) -> ApiResult<Json<maidan_types::PlanRelease>> {
    cap(&auth, maidan_auth::capability::TOKEN_ADMIN)?;
    let parent = ThreadId(id);
    let ctx = maidan_auth::authorize_thread(state.store.as_ref(), &auth, parent).await?;
    let released = state
        .store
        .release_plan(
            parent,
            maidan_types::NewAuditEvent {
                scope: maidan_types::AuditScope::Workspace(ctx.workspace_id),
                actor_id: Some(auth.actor_id),
                action: "plan.release".into(),
                target_kind: Some("thread".into()),
                target_id: Some(parent.0),
                metadata: serde_json::json!({}),
            },
        )
        .await?;
    Ok(Json(maidan_types::PlanRelease { released }))
}

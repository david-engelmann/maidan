//! Claim footprints over REST (see [`maidan_types::footprint`]): declare what
//! part of a repository a thread's work touches, read it back, and list the
//! live claims a swarm is working under.

use axum::{extract::State, Extension, Json};
use maidan_auth::capability::{THREAD_TRANSITION, WORKSPACE_READ};
use maidan_auth::AuthContext;
use maidan_types::{ActiveClaim, Footprint, ThreadId, WorkspaceId};

use crate::dto::ActiveClaimsQuery;
use crate::error::ApiError;
use crate::extract::{ApiJson, ApiPath, ApiQuery};
use crate::routes::{cap, ensure_workspace, ApiResult};
use crate::state::AppState;

/// Replace a thread's footprint; an empty `items` clears it. On a claimed
/// thread, a footprint that overlaps another live claim is a 409 naming the
/// overlapped claims, and nothing is stored. `thread:transition` + thread
/// access: a footprint is part of managing the task, as its result is.
pub async fn set_thread_footprint(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
    ApiJson(body): ApiJson<Footprint>,
) -> ApiResult<Json<Option<Footprint>>> {
    cap(&auth, THREAD_TRANSITION)?;
    let thread_id = ThreadId(id);
    maidan_auth::authorize_thread(state.store.as_ref(), &auth, thread_id).await?;
    let footprint = body.validated().map_err(ApiError::BadRequest)?;
    let footprint = (!footprint.items.is_empty()).then_some(footprint);
    let conflicts = state
        .store
        .set_thread_footprint(thread_id, footprint)
        .await?;
    if !conflicts.is_empty() {
        return Err(ApiError::FootprintConflict(conflicts));
    }
    Ok(Json(state.store.get_thread_footprint(thread_id).await?))
}

/// A thread's declared footprint, or `404` when it declares none.
/// `workspace:read` + thread access.
pub async fn get_thread_footprint(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
) -> ApiResult<Json<Footprint>> {
    cap(&auth, WORKSPACE_READ)?;
    let thread_id = ThreadId(id);
    maidan_auth::ensure_thread_access(state.store.as_ref(), &auth, thread_id).await?;
    state
        .store
        .get_thread_footprint(thread_id)
        .await?
        .map(Json)
        .ok_or(ApiError::NotFound)
}

/// The workspace's live claims with their footprints, in one repository when
/// `repo` is given. Only threads the caller may read are listed.
/// `workspace:read`.
pub async fn list_active_claims(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
    ApiQuery(query): ApiQuery<ActiveClaimsQuery>,
) -> ApiResult<Json<Vec<ActiveClaim>>> {
    cap(&auth, WORKSPACE_READ)?;
    let workspace_id = WorkspaceId(id);
    ensure_workspace(&auth, workspace_id)?;
    let repo = query
        .repo
        .map(|repo| maidan_types::normalize_repo(&repo).map_err(ApiError::BadRequest))
        .transpose()?;
    let readable_by = (!auth.bypass).then_some(auth.member_id);
    Ok(Json(
        state
            .store
            .list_active_claims(workspace_id, repo, readable_by)
            .await?,
    ))
}

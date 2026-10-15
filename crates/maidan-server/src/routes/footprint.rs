//! Claim footprints over REST (see [`maidan_types::footprint`]): declare what
//! part of a repository a thread's work touches, read it back, list the live
//! claims a swarm is working under, pause a repository so the queue hands
//! out none of its work, and cap how many claims hold a slot at once.

use axum::{extract::State, http::StatusCode, Extension, Json};
use maidan_auth::capability::{THREAD_TRANSITION, TOKEN_ADMIN, WORKSPACE_READ};
use maidan_auth::AuthContext;
use maidan_types::{
    ActiveClaim, AuditScope, DeferredThread, Footprint, NewAuditEvent, PauseRepo, RepoPause,
    SetSlotCapacity, SlotUsage, ThreadId, WorkspaceId,
};

use crate::dto::{ActiveClaimsQuery, RepoPauseQuery, SlotQuery};
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

/// The repositories paused in a workspace. `workspace:read`: a worker may
/// look before it plans work the queue would hold back.
pub async fn list_repo_pauses(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
) -> ApiResult<Json<Vec<RepoPause>>> {
    cap(&auth, WORKSPACE_READ)?;
    let workspace_id = WorkspaceId(id);
    ensure_workspace(&auth, workspace_id)?;
    Ok(Json(state.store.list_repo_pauses(workspace_id).await?))
}

/// Pause a repository: `claim_next` defers every thread whose footprint
/// names it, and claims already held run on. `token:admin`, as freezing a
/// member is: both stop agents taking work. Audited.
pub async fn pause_repo(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
    ApiJson(body): ApiJson<PauseRepo>,
) -> ApiResult<Json<RepoPause>> {
    cap(&auth, TOKEN_ADMIN)?;
    let workspace_id = WorkspaceId(id);
    ensure_workspace(&auth, workspace_id)?;
    let repo = maidan_types::normalize_repo(&body.repo).map_err(ApiError::BadRequest)?;
    let reason = body
        .reason
        .as_deref()
        .map(str::trim)
        .filter(|r| !r.is_empty());
    let actor = auth.actor_id;
    let pause = state
        .store
        .pause_repo_audited(
            workspace_id,
            &repo,
            auth.member_id,
            reason,
            Box::new(move |pause| NewAuditEvent {
                scope: AuditScope::Workspace(workspace_id),
                actor_id: Some(actor),
                action: "repo.pause".into(),
                target_kind: Some("repo".into()),
                target_id: None,
                metadata: serde_json::json!({ "repo": pause.repo, "reason": pause.reason }),
            }),
        )
        .await?;
    Ok(Json(pause))
}

/// Lift a repository pause: `204`, or `404` when it was not paused.
/// `token:admin`. Audited.
pub async fn resume_repo(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
    ApiQuery(query): ApiQuery<RepoPauseQuery>,
) -> ApiResult<StatusCode> {
    cap(&auth, TOKEN_ADMIN)?;
    let workspace_id = WorkspaceId(id);
    ensure_workspace(&auth, workspace_id)?;
    let repo = maidan_types::normalize_repo(&query.repo).map_err(ApiError::BadRequest)?;
    let removed = state
        .store
        .resume_repo_audited(
            workspace_id,
            &repo,
            NewAuditEvent {
                scope: AuditScope::Workspace(workspace_id),
                actor_id: Some(auth.actor_id),
                action: "repo.resume".into(),
                target_kind: Some("repo".into()),
                target_id: None,
                metadata: serde_json::json!({ "repo": repo }),
            },
        )
        .await?;
    if removed {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

/// The workspace's slots: each with a capacity, and each held by a live
/// claim, with how many hold it. `workspace:read`.
pub async fn list_slots(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
) -> ApiResult<Json<Vec<SlotUsage>>> {
    cap(&auth, WORKSPACE_READ)?;
    let workspace_id = WorkspaceId(id);
    ensure_workspace(&auth, workspace_id)?;
    Ok(Json(state.store.list_slots(workspace_id).await?))
}

/// Set how many live claims may hold a slot at once: `claim_next` defers a
/// thread whose slot is full, and claims already held run on. `token:admin`,
/// as pausing a repository is. Audited.
pub async fn set_slot_capacity(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
    ApiJson(body): ApiJson<SetSlotCapacity>,
) -> ApiResult<Json<SlotUsage>> {
    cap(&auth, TOKEN_ADMIN)?;
    let workspace_id = WorkspaceId(id);
    ensure_workspace(&auth, workspace_id)?;
    let slot = maidan_types::normalize_slot(&body.slot).map_err(ApiError::BadRequest)?;
    if body.capacity < 1 {
        return Err(ApiError::BadRequest("capacity is at least 1".into()));
    }
    state
        .store
        .set_slot_capacity_audited(
            workspace_id,
            &slot,
            body.capacity,
            auth.member_id,
            NewAuditEvent {
                scope: AuditScope::Workspace(workspace_id),
                actor_id: Some(auth.actor_id),
                action: "slot.capacity.set".into(),
                target_kind: Some("slot".into()),
                target_id: None,
                metadata: serde_json::json!({ "slot": slot, "capacity": body.capacity }),
            },
        )
        .await?;
    let usage = state
        .store
        .list_slots(workspace_id)
        .await?
        .into_iter()
        .find(|usage| usage.slot == slot)
        .unwrap_or(SlotUsage {
            slot,
            capacity: Some(body.capacity),
            in_use: 0,
        });
    Ok(Json(usage))
}

/// Remove a slot's capacity, so it limits nothing: `204`, or `404` when it
/// had none. `token:admin`. Audited.
pub async fn clear_slot_capacity(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
    ApiQuery(query): ApiQuery<SlotQuery>,
) -> ApiResult<StatusCode> {
    cap(&auth, TOKEN_ADMIN)?;
    let workspace_id = WorkspaceId(id);
    ensure_workspace(&auth, workspace_id)?;
    let slot = maidan_types::normalize_slot(&query.slot).map_err(ApiError::BadRequest)?;
    let removed = state
        .store
        .clear_slot_capacity_audited(
            workspace_id,
            &slot,
            NewAuditEvent {
                scope: AuditScope::Workspace(workspace_id),
                actor_id: Some(auth.actor_id),
                action: "slot.capacity.clear".into(),
                target_kind: Some("slot".into()),
                target_id: None,
                metadata: serde_json::json!({ "slot": slot }),
            },
        )
        .await?;
    if removed {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

/// The open threads the claim queue is holding back for their footprint,
/// oldest first, each with why: an overlap with a live claim, a paused
/// repository, or a full slot. Only threads the caller may read are listed.
/// `workspace:read`.
pub async fn list_deferred_threads(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(id): ApiPath<uuid::Uuid>,
) -> ApiResult<Json<Vec<DeferredThread>>> {
    cap(&auth, WORKSPACE_READ)?;
    let workspace_id = WorkspaceId(id);
    ensure_workspace(&auth, workspace_id)?;
    let readable_by = (!auth.bypass).then_some(auth.member_id);
    Ok(Json(
        state
            .store
            .list_deferred_threads(workspace_id, readable_by)
            .await?,
    ))
}

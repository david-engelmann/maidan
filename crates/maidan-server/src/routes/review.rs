//! Required-reviewers management. A thread declares a review requirement (`k`
//! approvals) from a named reviewer set; a reviewer submits an approve /
//! request-changes decision. The FSM close-gate then refuses `closed` until `k`
//! qualifying approvals exist and no `refutes` edge blocks the thread.
//! Governance writes are `thread:transition`; reads are `workspace:read`.
//!
//! **A gate ratchets**. Making the requirement *stricter* is
//! `thread:transition`; making it looser is `channel:admin`, and audited.
//! Loosening is exactly as powerful as closing — and `thread:transition` is
//! what a close needs and what `maidan.agent.worker` carries, so guarding both
//! with it let the constrained agent dissolve its own gate. Three operations
//! loosen: clearing the requirement, lowering `k`, and un-designating a
//! reviewer (an empty named set means *any* non-implementer approval counts).

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use maidan_auth::{
    capability::{CHANNEL_ADMIN, THREAD_TRANSITION, WORKSPACE_READ},
    AuthContext,
};
use maidan_types::*;

use super::{cap, ApiResult};
use crate::dto::*;
use crate::error::{ApiError, ApiJson};
use crate::state::AppState;

pub async fn set_review_requirement(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<SetReviewRequirement>,
) -> ApiResult<Json<ThreadReviewRequirement>> {
    cap(&auth, THREAD_TRANSITION)?;
    let thread_id = ThreadId(id);
    maidan_auth::authorize_thread(state.store.as_ref(), &auth, thread_id).await?;
    if body.required_count < 0 {
        return Err(ApiError::BadRequest("required_count must be >= 0".into()));
    }
    // Raising `k` is a tightening any transitioner may do. Lowering it — `0`
    // included, which disarms the gate — is the waiver.
    let current = state
        .store
        .get_review_requirement(thread_id)
        .await?
        .map(|r| r.required_count)
        .unwrap_or(0);
    if body.required_count < current {
        cap(&auth, CHANNEL_ADMIN)?;
        crate::audit::record(
            &state,
            NewAuditEvent {
                actor_id: Some(auth.member_id),
                action: "review_requirement.lower".into(),
                target_kind: Some("thread".into()),
                target_id: Some(thread_id.0),
                metadata: serde_json::json!({ "from": current, "to": body.required_count }),
            },
        )
        .await;
    }
    let req = state
        .store
        .set_review_requirement(thread_id, body.required_count)
        .await?;
    Ok(Json(req))
}

pub async fn get_review_requirement(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<ThreadReviewRequirement>> {
    cap(&auth, WORKSPACE_READ)?;
    let thread_id = ThreadId(id);
    maidan_auth::authorize_thread(state.store.as_ref(), &auth, thread_id).await?;
    state
        .store
        .get_review_requirement(thread_id)
        .await?
        .map(Json)
        .ok_or(ApiError::NotFound)
}

/// Remove the requirement — `channel:admin`, audited. The gate only fires when
/// `required_count > 0`, so deleting the row is a full waiver.
pub async fn clear_review_requirement(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<StatusCode> {
    cap(&auth, CHANNEL_ADMIN)?;
    let thread_id = ThreadId(id);
    maidan_auth::authorize_thread(state.store.as_ref(), &auth, thread_id).await?;
    if state.store.clear_review_requirement(thread_id).await? {
        crate::audit::record(
            &state,
            NewAuditEvent {
                actor_id: Some(auth.member_id),
                action: "review_requirement.clear".into(),
                target_kind: Some("thread".into()),
                target_id: Some(thread_id.0),
                metadata: serde_json::json!({}),
            },
        )
        .await;
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

pub async fn add_reviewer(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<AddReviewer>,
) -> ApiResult<StatusCode> {
    cap(&auth, THREAD_TRANSITION)?;
    let thread_id = ThreadId(id);
    let ctx = maidan_auth::authorize_thread(state.store.as_ref(), &auth, thread_id).await?;
    let member = MemberId(body.member_id);
    // The reviewer must be a member of the thread's workspace.
    let m = state.store.get_member(member).await?;
    if m.workspace_id != ctx.workspace_id {
        return Err(ApiError::BadRequest(
            "reviewer is not in this workspace".into(),
        ));
    }
    state.store.add_reviewer(thread_id, member).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn remove_reviewer(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path((id, member_id)): Path<(uuid::Uuid, uuid::Uuid)>,
) -> ApiResult<StatusCode> {
    // Un-designating is `channel:admin` unconditionally rather than only when it
    // empties the set: "is this the last one?" is a read-then-write, so two
    // concurrent removes could each see a survivor and still empty it together.
    cap(&auth, CHANNEL_ADMIN)?;
    let thread_id = ThreadId(id);
    maidan_auth::authorize_thread(state.store.as_ref(), &auth, thread_id).await?;
    if state
        .store
        .remove_reviewer(thread_id, MemberId(member_id))
        .await?
    {
        crate::audit::record(
            &state,
            NewAuditEvent {
                actor_id: Some(auth.member_id),
                action: "reviewer.remove".into(),
                target_kind: Some("thread".into()),
                target_id: Some(thread_id.0),
                metadata: serde_json::json!({ "member_id": member_id }),
            },
        )
        .await;
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

pub async fn list_reviewers(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<Vec<MemberId>>> {
    cap(&auth, WORKSPACE_READ)?;
    let thread_id = ThreadId(id);
    maidan_auth::authorize_thread(state.store.as_ref(), &auth, thread_id).await?;
    Ok(Json(state.store.list_reviewers(thread_id).await?))
}

pub async fn submit_review(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<SubmitReview>,
) -> ApiResult<Json<ThreadReview>> {
    cap(&auth, THREAD_TRANSITION)?;
    let thread_id = ThreadId(id);
    maidan_auth::authorize_thread(state.store.as_ref(), &auth, thread_id).await?;
    // The reviewer is the caller; an owner/assignee may submit but it won't count.
    let note = body
        .note
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty());
    let review = state
        .store
        .submit_review(thread_id, auth.member_id, body.decision, note)
        .await?;
    Ok(Json(review))
}

pub async fn list_reviews(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<Vec<ThreadReview>>> {
    cap(&auth, WORKSPACE_READ)?;
    let thread_id = ThreadId(id);
    maidan_auth::authorize_thread(state.store.as_ref(), &auth, thread_id).await?;
    Ok(Json(state.store.list_reviews(thread_id).await?))
}

pub async fn get_review_status(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<ReviewStatus>> {
    cap(&auth, WORKSPACE_READ)?;
    let thread_id = ThreadId(id);
    maidan_auth::authorize_thread(state.store.as_ref(), &auth, thread_id).await?;
    Ok(Json(state.store.review_status(thread_id).await?))
}

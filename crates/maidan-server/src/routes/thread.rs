//! Thread handlers: create/list/get threads, thread context, and FSM
//! transitions.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Extension, Json,
};
use chrono::Utc;
use maidan_auth::{
    capability::{THREAD_TRANSITION, WORKSPACE_READ, WORKSPACE_WRITE},
    AuthContext,
};
use maidan_fsm::ThreadAction;
use maidan_router::{resolve_channel_context, resolve_thread_context};
use maidan_types::*;

use super::{cap, ensure_workspace, publish, ApiResult};
use crate::dto::*;
use crate::error::{ApiError, ApiJson};
use crate::state::AppState;

pub async fn create_thread(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(channel_id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<CreateThread>,
) -> ApiResult<(StatusCode, Json<Thread>)> {
    let ctx = resolve_channel_context(state.store.as_ref(), ChannelId(channel_id)).await?;
    cap(&auth, WORKSPACE_WRITE)?;
    ensure_workspace(&auth, ctx.workspace_id)?;
    maidan_auth::ensure_channel_access(state.store.as_ref(), &auth, ctx.channel_id).await?;
    let t = state
        .store
        .create_thread(NewThread {
            channel_id: ChannelId(channel_id),
            parent_thread_id: body.parent_thread_id.map(ThreadId),
            title: body.title,
        })
        .await?;
    publish(
        &state,
        Event::ThreadCreated {
            occurred_at: Utc::now(),
            workspace_id: ctx.workspace_id,
            channel_id: ChannelId(channel_id),
            thread: t.clone(),
        },
    )
    .await;
    Ok((StatusCode::CREATED, Json(t)))
}

pub async fn list_threads(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(channel_id): Path<uuid::Uuid>,
) -> ApiResult<Json<Vec<Thread>>> {
    let ctx = resolve_channel_context(state.store.as_ref(), ChannelId(channel_id)).await?;
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, ctx.workspace_id)?;
    maidan_auth::ensure_channel_access(state.store.as_ref(), &auth, ctx.channel_id).await?;
    Ok(Json(state.store.list_threads(ChannelId(channel_id)).await?))
}

pub async fn get_thread(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<Thread>> {
    let thread = state.store.get_thread(ThreadId(id)).await?;
    let ctx = resolve_thread_context(state.store.as_ref(), ThreadId(id)).await?;
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, ctx.workspace_id)?;
    maidan_auth::ensure_channel_access(state.store.as_ref(), &auth, ctx.channel_id).await?;
    Ok(Json(thread))
}

pub async fn get_thread_context(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
    Query(q): Query<ThreadContextQuery>,
) -> ApiResult<Json<crate::thread_context::ThreadContext>> {
    let thread_id = ThreadId(id);
    let ctx = resolve_thread_context(state.store.as_ref(), thread_id).await?;
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, ctx.workspace_id)?;
    maidan_auth::ensure_channel_access(state.store.as_ref(), &auth, ctx.channel_id).await?;
    let packed = crate::thread_context::build_thread_context(
        state.store.as_ref(),
        thread_id,
        crate::thread_context::ThreadContextLimits {
            message_limit: if q.message_limit > 0 {
                q.message_limit
            } else {
                100
            },
            transition_limit: if q.transition_limit > 0 {
                q.transition_limit
            } else {
                50
            },
            message_cursor: q.message_cursor.map(MessageId),
            include_edits: q.include_edits,
        },
    )
    .await?;
    Ok(Json(packed))
}

pub async fn transition_thread(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<TransitionThread>,
) -> ApiResult<Json<Thread>> {
    cap(&auth, THREAD_TRANSITION)?;
    let action = ThreadAction::parse(&body.action).ok_or_else(|| {
        ApiError::BadRequest(format!(
            "unknown action {:?}; expected start_review, close, or archive",
            body.action
        ))
    })?;
    let thread_id = ThreadId(id);
    let ctx = resolve_thread_context(state.store.as_ref(), thread_id).await?;
    let workspace_id = ctx.workspace_id;
    let channel_id = ctx.channel_id;
    ensure_workspace(&auth, workspace_id)?;
    maidan_auth::ensure_channel_access(state.store.as_ref(), &auth, channel_id).await?;
    let result = state
        .store
        .transition_thread(thread_id, MemberId(body.actor_id), action)
        .await?;
    publish(
        &state,
        Event::ThreadStateChanged {
            occurred_at: Utc::now(),
            workspace_id,
            channel_id,
            thread_id,
            actor_id: MemberId(body.actor_id),
            from_state: result.from_state,
            to_state: result.to_state,
            thread: result.thread.clone(),
        },
    )
    .await;
    let uris =
        maidan_mcp::resource_updates::uris_for_thread_transition(state.store.as_ref(), thread_id)
            .await;
    state.mcp.publish_resource_uris(uris).await;
    Ok(Json(result.thread))
}

/// Publish a `ThreadAssignmentChanged` event (Cluster 171). Shared by the
/// assign / claim / unassign handlers.
async fn publish_assignment(
    state: &AppState,
    workspace_id: WorkspaceId,
    channel_id: ChannelId,
    thread: &Thread,
    actor_id: MemberId,
    previous_assignee_id: Option<MemberId>,
) {
    publish(
        state,
        Event::ThreadAssignmentChanged {
            occurred_at: Utc::now(),
            workspace_id,
            channel_id,
            thread_id: thread.id,
            actor_id,
            previous_assignee_id,
            assignee_id: thread.assignee_id,
            thread: thread.clone(),
        },
    )
    .await;
}

pub async fn assign_thread(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<AssignThread>,
) -> ApiResult<Json<Thread>> {
    cap(&auth, THREAD_TRANSITION)?;
    let thread_id = ThreadId(id);
    let ctx = resolve_thread_context(state.store.as_ref(), thread_id).await?;
    ensure_workspace(&auth, ctx.workspace_id)?;
    maidan_auth::ensure_channel_access(state.store.as_ref(), &auth, ctx.channel_id).await?;
    let previous = state.store.get_thread(thread_id).await?.assignee_id;
    let thread = state
        .store
        .assign_thread(thread_id, MemberId(body.assignee_id))
        .await?;
    publish_assignment(
        &state,
        ctx.workspace_id,
        ctx.channel_id,
        &thread,
        MemberId(body.actor_id),
        previous,
    )
    .await;
    Ok(Json(thread))
}

pub async fn unassign_thread(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<UnassignThread>,
) -> ApiResult<Json<Thread>> {
    cap(&auth, THREAD_TRANSITION)?;
    let thread_id = ThreadId(id);
    let ctx = resolve_thread_context(state.store.as_ref(), thread_id).await?;
    ensure_workspace(&auth, ctx.workspace_id)?;
    maidan_auth::ensure_channel_access(state.store.as_ref(), &auth, ctx.channel_id).await?;
    let previous = state.store.get_thread(thread_id).await?.assignee_id;
    let thread = state.store.unassign_thread(thread_id).await?;
    publish_assignment(
        &state,
        ctx.workspace_id,
        ctx.channel_id,
        &thread,
        MemberId(body.actor_id),
        previous,
    )
    .await;
    Ok(Json(thread))
}

pub async fn claim_thread(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<ClaimThread>,
) -> ApiResult<Json<ThreadClaimResult>> {
    cap(&auth, THREAD_TRANSITION)?;
    let thread_id = ThreadId(id);
    let ctx = resolve_thread_context(state.store.as_ref(), thread_id).await?;
    ensure_workspace(&auth, ctx.workspace_id)?;
    maidan_auth::ensure_channel_access(state.store.as_ref(), &auth, ctx.channel_id).await?;
    let member_id = MemberId(body.member_id);
    let result = state.store.claim_thread(thread_id, member_id).await?;
    if result.claimed {
        publish_assignment(
            &state,
            ctx.workspace_id,
            ctx.channel_id,
            &result.thread,
            member_id,
            None,
        )
        .await;
    }
    Ok(Json(result))
}

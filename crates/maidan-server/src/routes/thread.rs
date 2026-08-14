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

use super::{cap, ensure_workspace, publish, publish_stored, ApiResult};
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
    // Cluster 205: the thread row and its `ThreadCreated` event commit atomically
    // (transactional outbox); `publish_stored` then notifies the bus.
    let (t, stored) = state
        .store
        .create_thread_with_event(NewThread {
            channel_id: ChannelId(channel_id),
            parent_thread_id: body.parent_thread_id.map(ThreadId),
            title: body.title,
        })
        .await?;
    publish_stored(&state, stored).await;
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
    // Cluster 180: thread-scoped access (DM-participant-aware for `__dm__`),
    // not channel-scoped — the generic route must not expose a DM thread to a
    // non-participant.
    maidan_auth::ensure_thread_access(state.store.as_ref(), &auth, ThreadId(id)).await?;
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
    maidan_auth::ensure_thread_access(state.store.as_ref(), &auth, thread_id).await?;
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

/// `GET /threads/:id/tool-transcript` (Cluster 197) — the thread's tool-call
/// transcript: every `ToolUse` block correlated with its `ToolResult` by id. A
/// token-lean projection that drops text/code blocks and message bodies.
pub async fn get_tool_transcript(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
    Query(q): Query<ToolTranscriptQuery>,
) -> ApiResult<Json<ToolTranscript>> {
    let thread_id = ThreadId(id);
    let ctx = resolve_thread_context(state.store.as_ref(), thread_id).await?;
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, ctx.workspace_id)?;
    maidan_auth::ensure_thread_access(state.store.as_ref(), &auth, thread_id).await?;
    let limit = q.limit.unwrap_or(200).clamp(1, 500);
    let messages = state.store.list_messages(thread_id, limit).await?;
    Ok(Json(tool_transcript(thread_id, &messages)))
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
    ensure_workspace(&auth, ctx.workspace_id)?;
    maidan_auth::ensure_thread_access(state.store.as_ref(), &auth, thread_id).await?;
    super::ensure_acting_member(&auth, MemberId(body.actor_id))?;
    let (result, stored) = state
        .store
        .transition_thread_with_event(thread_id, MemberId(body.actor_id), action)
        .await?;
    super::publish_stored(&state, stored).await;
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
    note: Option<String>,
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
            note,
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
    maidan_auth::ensure_thread_access(state.store.as_ref(), &auth, thread_id).await?;
    super::ensure_acting_member(&auth, MemberId(body.actor_id))?;
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
        body.note,
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
    maidan_auth::ensure_thread_access(state.store.as_ref(), &auth, thread_id).await?;
    super::ensure_acting_member(&auth, MemberId(body.actor_id))?;
    let previous = state.store.get_thread(thread_id).await?.assignee_id;
    let thread = state.store.unassign_thread(thread_id).await?;
    publish_assignment(
        &state,
        ctx.workspace_id,
        ctx.channel_id,
        &thread,
        MemberId(body.actor_id),
        previous,
        None,
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
    maidan_auth::ensure_thread_access(state.store.as_ref(), &auth, thread_id).await?;
    let member_id = MemberId(body.member_id);
    super::ensure_acting_member(&auth, member_id)?;
    let result = state.store.claim_thread(thread_id, member_id).await?;
    if result.claimed {
        publish_assignment(
            &state,
            ctx.workspace_id,
            ctx.channel_id,
            &result.thread,
            member_id,
            None,
            None,
        )
        .await;
    }
    Ok(Json(result))
}

/// A member's work queue: threads assigned to them (Cluster 190). Filtered to
/// threads the *caller* can access (RBAC-consistent with search / context).
pub async fn list_assigned_threads(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<Vec<Thread>>> {
    cap(&auth, WORKSPACE_READ)?;
    let member_id = MemberId(id);
    let member = state.store.get_member(member_id).await?;
    ensure_workspace(&auth, member.workspace_id)?;
    let threads = state
        .store
        .list_assigned_threads(member.workspace_id, member_id)
        .await?;
    if auth.bypass {
        return Ok(Json(threads));
    }
    let mut visible = Vec::with_capacity(threads.len());
    for t in threads {
        if maidan_auth::can_access_thread(state.store.as_ref(), &auth, t.id).await? {
            visible.push(t);
        }
    }
    Ok(Json(visible))
}

/// Atomically claim the oldest unassigned thread in a channel (Cluster 190) —
/// the "pull the next task" primitive. Returns the claimed thread, or `null`
/// when the channel has no unassigned work.
pub async fn claim_next_thread(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(cid): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<ClaimNextThread>,
) -> ApiResult<Json<Option<Thread>>> {
    cap(&auth, THREAD_TRANSITION)?;
    let channel = state.store.get_channel(ChannelId(cid)).await?;
    ensure_workspace(&auth, channel.workspace_id)?;
    maidan_auth::ensure_channel_access(state.store.as_ref(), &auth, channel.id).await?;
    let member_id = MemberId(body.member_id);
    super::ensure_acting_member(&auth, member_id)?;
    let claimed = state
        .store
        .claim_next_thread(channel.id, member_id, body.lease_secs)
        .await?;
    if let Some(thread) = &claimed {
        publish_assignment(
            &state,
            channel.workspace_id,
            channel.id,
            thread,
            member_id,
            None,
            None,
        )
        .await;
    }
    Ok(Json(claimed))
}

/// Extend a claimed thread's lease (heartbeat), for the current assignee only
/// (Cluster 192). `NotFound` if the caller isn't the holder.
pub async fn renew_claim(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<RenewClaim>,
) -> ApiResult<Json<Thread>> {
    cap(&auth, THREAD_TRANSITION)?;
    let thread_id = ThreadId(id);
    let ctx = resolve_thread_context(state.store.as_ref(), thread_id).await?;
    ensure_workspace(&auth, ctx.workspace_id)?;
    maidan_auth::ensure_thread_access(state.store.as_ref(), &auth, thread_id).await?;
    super::ensure_acting_member(&auth, MemberId(body.member_id))?;
    let thread = state
        .store
        .renew_claim(thread_id, MemberId(body.member_id), body.lease_secs)
        .await?;
    Ok(Json(thread))
}

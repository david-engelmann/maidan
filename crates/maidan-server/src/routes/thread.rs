//! Thread handlers: create/list/get threads, thread context, and FSM
//! transitions.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Extension, Json,
};
use maidan_auth::{
    capability::{ARTIFACT_UPLOAD, THREAD_TRANSITION, WORKSPACE_READ, WORKSPACE_WRITE},
    AuthContext,
};
use maidan_fsm::ThreadAction;
use maidan_router::{resolve_channel_context, resolve_thread_context};
use maidan_types::*;

use super::{cap, ensure_workspace, publish_stored, ApiResult};
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
            include_glossary: q.include_glossary,
            as_of: q.as_of,
        },
    )
    .await?;
    Ok(Json(packed))
}

/// `POST /threads/:id/context/snapshot` — freeze the assembled context pack (live
/// or `as_of`) into the content-addressed artifact store (Cluster 329): a
/// tamper-evident record of exactly what the agent was handed. Deduped by sha256
/// (identical packs share a blob) and ref-guarded per Cluster 204. Re-ask can
/// later attach the snapshot by its sha. Gated `artifact:upload` + thread access.
pub async fn snapshot_thread_context(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
    Query(q): Query<ThreadContextQuery>,
) -> ApiResult<(StatusCode, Json<Artifact>)> {
    let thread_id = ThreadId(id);
    let ctx = resolve_thread_context(state.store.as_ref(), thread_id).await?;
    cap(&auth, ARTIFACT_UPLOAD)?;
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
            include_glossary: q.include_glossary,
            as_of: q.as_of,
        },
    )
    .await?;
    let body: axum::body::Bytes = serde_json::to_vec(&packed)
        .map_err(|e| ApiError::Internal(format!("serialize context snapshot: {e}")))?
        .into();
    let size_bytes = body.len() as i64;
    let sha = state.artifacts.put(body).await?;
    // Same atomic upsert + Cluster-204 per-workspace ref + `ArtifactUpserted` event
    // as a normal upload; the ref/uploader are recorded only for a non-bypass caller.
    let ref_workspace = (!auth.bypass).then_some(auth.workspace_id);
    let uploaded_by = (!auth.bypass).then_some(auth.member_id);
    let (artifact, stored) = state
        .store
        .upsert_artifact_with_event(
            NewArtifact {
                sha256: sha.to_string(),
                size_bytes,
                mime_type: Some("application/json".to_string()),
                kind: ArtifactKind::ContextSnapshot,
                uploaded_by,
            },
            ref_workspace,
        )
        .await?;
    publish_stored(&state, stored).await;
    Ok((StatusCode::CREATED, Json(artifact)))
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
    // Cluster 222: entering a terminal state can unblock dependents. Push a
    // `ThreadReady` for each task that just became ready, so an agent waiting on
    // the DAG needn't poll `dependencies_satisfied`. Derived + best-effort: a
    // failed emit doesn't undo the committed transition (readiness stays
    // queryable), so a store error here is logged, not surfaced.
    if !result.from_state.is_terminal() && result.to_state.is_terminal() {
        match state.store.newly_ready_dependents(thread_id).await {
            Ok(ready) => {
                for dep in ready {
                    super::publish(
                        &state,
                        Event::ThreadReady {
                            occurred_at: chrono::Utc::now(),
                            workspace_id: ctx.workspace_id,
                            channel_id: dep.channel_id,
                            thread_id: dep.id,
                            thread: dep,
                        },
                    )
                    .await;
                }
            }
            Err(err) => {
                tracing::warn!(error = %err, "thread_ready: newly_ready_dependents failed");
            }
        }
    }
    let uris =
        maidan_mcp::resource_updates::uris_for_thread_transition(state.store.as_ref(), thread_id)
            .await;
    state.mcp.publish_resource_uris(uris).await;
    Ok(Json(result.thread))
}

/// Set (upsert) a task's structured result (Cluster 235). `thread:transition` +
/// thread access — producing a task's output is managing the task. Emits a
/// `ThreadResultSet` event so waiters (a parent task, `wait_for_result`) can react.
pub async fn set_thread_result(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<SetThreadResult>,
) -> ApiResult<Json<ThreadResult>> {
    cap(&auth, THREAD_TRANSITION)?;
    let thread_id = ThreadId(id);
    let ctx = resolve_thread_context(state.store.as_ref(), thread_id).await?;
    ensure_workspace(&auth, ctx.workspace_id)?;
    maidan_auth::ensure_thread_access(state.store.as_ref(), &auth, thread_id).await?;
    let result = state
        .store
        .set_thread_result(thread_id, auth.member_id, &body.result)
        .await?;
    super::publish(
        &state,
        Event::ThreadResultSet {
            occurred_at: chrono::Utc::now(),
            workspace_id: ctx.workspace_id,
            channel_id: ctx.channel_id,
            thread_id,
            produced_by: auth.member_id,
        },
    )
    .await;
    Ok(Json(result))
}

/// A task's structured result, or `404` if none has been produced (Cluster 235).
/// `workspace:read` + thread access.
pub async fn get_thread_result(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<ThreadResult>> {
    cap(&auth, WORKSPACE_READ)?;
    let thread_id = ThreadId(id);
    let ctx = resolve_thread_context(state.store.as_ref(), thread_id).await?;
    ensure_workspace(&auth, ctx.workspace_id)?;
    maidan_auth::ensure_thread_access(state.store.as_ref(), &auth, thread_id).await?;
    match state.store.get_thread_result(thread_id).await? {
        Some(result) => Ok(Json(result)),
        None => Err(ApiError::NotFound),
    }
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
    let (thread, stored) = state
        .store
        .assign_thread_with_event(
            thread_id,
            MemberId(body.assignee_id),
            MemberId(body.actor_id),
            body.note,
        )
        .await?;
    super::publish_stored(&state, stored).await;
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
    let (thread, stored) = state
        .store
        .unassign_thread_with_event(thread_id, MemberId(body.actor_id))
        .await?;
    super::publish_stored(&state, stored).await;
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
    let (result, stored) = state
        .store
        .claim_thread_with_event(thread_id, member_id)
        .await?;
    if let Some(stored) = stored {
        super::publish_stored(&state, stored).await;
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
    let (claimed, stored) = state
        .store
        .claim_next_thread_with_event(channel.id, member_id, body.lease_secs)
        .await?;
    if let Some(stored) = stored {
        super::publish_stored(&state, stored).await;
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

/// Add a task-dependency edge (Cluster 219): the path thread depends on
/// `depends_on_thread_id`. Both threads must be in the same workspace and visible
/// to the caller. `thread:transition` — dependency wiring is a workflow op.
pub async fn add_thread_dependency(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<AddThreadDependency>,
) -> ApiResult<StatusCode> {
    cap(&auth, THREAD_TRANSITION)?;
    let thread_id = ThreadId(id);
    let depends_on = ThreadId(body.depends_on_thread_id);
    let ctx = resolve_thread_context(state.store.as_ref(), thread_id).await?;
    ensure_workspace(&auth, ctx.workspace_id)?;
    maidan_auth::ensure_thread_access(state.store.as_ref(), &auth, thread_id).await?;
    // The dependency must be in the same workspace and visible to the caller too.
    let dep_ctx = resolve_thread_context(state.store.as_ref(), depends_on).await?;
    if dep_ctx.workspace_id != ctx.workspace_id {
        return Err(ApiError::BadRequest(
            "dependency thread is in a different workspace".into(),
        ));
    }
    maidan_auth::ensure_thread_access(state.store.as_ref(), &auth, depends_on).await?;
    state
        .store
        .add_thread_dependency(thread_id, depends_on)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// A task's dependencies + whether it is ready (all deps terminal) — Cluster 219.
pub async fn list_thread_dependencies(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<ThreadDependenciesView>> {
    cap(&auth, WORKSPACE_READ)?;
    let thread_id = ThreadId(id);
    let ctx = resolve_thread_context(state.store.as_ref(), thread_id).await?;
    ensure_workspace(&auth, ctx.workspace_id)?;
    maidan_auth::ensure_thread_access(state.store.as_ref(), &auth, thread_id).await?;
    let dependencies = state.store.list_thread_dependencies(thread_id).await?;
    let ready = state.store.thread_dependencies_satisfied(thread_id).await?;
    Ok(Json(ThreadDependenciesView {
        dependencies,
        ready,
    }))
}

/// Remove a dependency edge (Cluster 219). `NotFound` if the edge doesn't exist.
pub async fn remove_thread_dependency(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path((id, dep_id)): Path<(uuid::Uuid, uuid::Uuid)>,
) -> ApiResult<StatusCode> {
    cap(&auth, THREAD_TRANSITION)?;
    let thread_id = ThreadId(id);
    let ctx = resolve_thread_context(state.store.as_ref(), thread_id).await?;
    ensure_workspace(&auth, ctx.workspace_id)?;
    maidan_auth::ensure_thread_access(state.store.as_ref(), &auth, thread_id).await?;
    if state
        .store
        .remove_thread_dependency(thread_id, ThreadId(dep_id))
        .await?
    {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::NotFound)
    }
}

/// The tasks blocked by this thread — its dependents (Cluster 219).
pub async fn list_thread_dependents(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<Vec<ThreadDependency>>> {
    cap(&auth, WORKSPACE_READ)?;
    let thread_id = ThreadId(id);
    let ctx = resolve_thread_context(state.store.as_ref(), thread_id).await?;
    ensure_workspace(&auth, ctx.workspace_id)?;
    maidan_auth::ensure_thread_access(state.store.as_ref(), &auth, thread_id).await?;
    Ok(Json(state.store.list_thread_dependents(thread_id).await?))
}

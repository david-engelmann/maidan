//! HTTP CRUD handlers. Every handler returns `Result<Json<_>, ApiError>`
//! and lets the [`crate::error::ApiError`] type render the failure as
//! `application/problem+json`. Mutations publish an [`Event`] to the
//! bus after the store call succeeds.

use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Extension, Json,
};
use chrono::Utc;
use maidan_artifacts::{ArtifactStore, CompletedPart, MultipartUpload, S3Store, Sha256};
use maidan_auth::{
    capability::{
        self, ARTIFACT_UPLOAD, MESSAGE_POST, SEARCH_QUERY, THREAD_TRANSITION, TOKEN_ADMIN,
        WORKSPACE_READ, WORKSPACE_WRITE,
    },
    hash_secret, AuthContext, TokenSecret,
};
use maidan_fsm::ThreadAction;
use maidan_router::{
    resolve_channel_context, resolve_message_chain, resolve_thread_context,
    route_mentions_for_message,
};
use maidan_types::*;

use crate::dto::*;
use crate::error::{ApiError, ApiJson};
use crate::federation::PeerContext;
use crate::state::AppState;

type ApiResult<T> = Result<T, ApiError>;

pub(crate) fn cap(auth: &AuthContext, capability: &str) -> ApiResult<()> {
    auth.require_capability(capability).map_err(Into::into)
}

pub(crate) fn ensure_workspace(auth: &AuthContext, workspace_id: WorkspaceId) -> ApiResult<()> {
    auth.ensure_workspace(workspace_id).map_err(Into::into)
}

fn ensure_message_edit(
    auth: &AuthContext,
    editor_id: MemberId,
    author_id: MemberId,
) -> ApiResult<()> {
    if auth.bypass {
        return Ok(());
    }
    if editor_id == author_id {
        cap(auth, MESSAGE_POST)
    } else {
        cap(auth, WORKSPACE_WRITE)
    }
}

// --- workspaces ---

pub async fn create_workspace(
    State(state): State<AppState>,
    ApiJson(body): ApiJson<CreateWorkspace>,
) -> ApiResult<(StatusCode, Json<Workspace>)> {
    if !state.auth_disabled && state.bootstrap_enabled {
        let count = state.store.count_workspaces().await?;
        if count > 0 {
            return Err(ApiError::Forbidden(
                "bootstrap only allows creating the first workspace; use bearer auth thereafter"
                    .into(),
            ));
        }
    }
    let ws = state
        .store
        .create_workspace(NewWorkspace { name: body.name })
        .await?;
    publish(
        &state,
        Event::WorkspaceCreated {
            occurred_at: Utc::now(),
            workspace: ws.clone(),
        },
    )
    .await;
    Ok((StatusCode::CREATED, Json(ws)))
}

pub async fn get_workspace(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<Workspace>> {
    let workspace_id = WorkspaceId(id);
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, workspace_id)?;
    Ok(Json(state.store.get_workspace(workspace_id).await?))
}

pub async fn replay_quarantined_outbox(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path((wid, outbox_id)): Path<(uuid::Uuid, i64)>,
) -> ApiResult<StatusCode> {
    let workspace_id = WorkspaceId(wid);
    cap(&auth, WORKSPACE_WRITE)?;
    ensure_workspace(&auth, workspace_id)?;
    let backend = state.outbox_backend.as_ref().ok_or_else(|| {
        ApiError::BadRequest("outbox relay is not enabled for this deployment".into())
    })?;
    backend.replay_quarantined(outbox_id, workspace_id).await?;
    let actor_id = if auth.bypass {
        None
    } else {
        Some(auth.member_id)
    };
    state
        .store
        .append_audit(NewAuditEvent {
            actor_id,
            action: "outbox.replay".into(),
            target_kind: Some("outbox".into()),
            target_id: None,
            metadata: serde_json::json!({
                "outbox_id": outbox_id,
                "workspace_id": workspace_id.0,
            }),
        })
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, serde::Deserialize)]
pub struct QuarantinedOutboxQuery {
    #[serde(default = "default_outbox_list_limit")]
    pub limit: i64,
}

fn default_outbox_list_limit() -> i64 {
    50
}

pub async fn list_quarantined_outbox(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(wid): Path<uuid::Uuid>,
    Query(q): Query<QuarantinedOutboxQuery>,
) -> ApiResult<Json<Vec<maidan_store::QuarantinedOutboxRow>>> {
    let workspace_id = WorkspaceId(wid);
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, workspace_id)?;
    let backend = state.outbox_backend.as_ref().ok_or_else(|| {
        ApiError::BadRequest("outbox relay is not enabled for this deployment".into())
    })?;
    let limit = q.limit.clamp(1, 500);
    let rows = backend
        .list_quarantined_for_workspace(workspace_id, limit)
        .await?;
    Ok(Json(rows))
}

pub async fn get_workspace_context(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(wid): Path<uuid::Uuid>,
    Query(q): Query<WorkspaceContextQuery>,
) -> ApiResult<Json<crate::thread_context::WorkspaceContext>> {
    let workspace_id = WorkspaceId(wid);
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, workspace_id)?;
    let limits = crate::thread_context::ThreadContextLimits {
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
        message_cursor: None,
    };
    let packed = crate::thread_context::build_workspace_context(
        state.store.as_ref(),
        workspace_id,
        q.thread_limit.clamp(1, 50),
        q.thread_cursor.map(ThreadId),
        limits,
    )
    .await?;
    Ok(Json(packed))
}

pub async fn purge_workspace(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<WorkspacePurgeResult>> {
    let workspace_id = WorkspaceId(id);
    cap(&auth, WORKSPACE_WRITE)?;
    ensure_workspace(&auth, workspace_id)?;
    state.store.get_workspace(workspace_id).await?;
    let mut result = state.store.purge_workspace_messages(workspace_id).await?;
    let mut artifact_blobs_deleted = 0u64;
    for sha_hex in &result.artifact_shas {
        let Ok(sha) = maidan_artifacts::Sha256::from_hex(sha_hex) else {
            continue;
        };
        if state.artifacts.delete(&sha).await.is_ok() {
            artifact_blobs_deleted += 1;
        }
    }
    result.artifact_shas.clear();
    state
        .store
        .append_audit(NewAuditEvent {
            actor_id: Some(auth.member_id),
            action: "workspace.purge".into(),
            target_kind: Some("workspace".into()),
            target_id: Some(workspace_id.0),
            metadata: serde_json::json!({
                "messages_tombstoned": result.messages_tombstoned,
                "messages_purged": result.messages_purged,
                "embeddings_removed": result.embeddings_removed,
                "references_removed": result.references_removed,
                "api_tokens_revoked": result.api_tokens_revoked,
                "events_removed": result.events_removed,
                "artifacts_removed": result.artifacts_removed,
                "artifact_blobs_deleted": artifact_blobs_deleted,
            }),
        })
        .await?;
    let uris = maidan_mcp::resource_updates::uris_for_workspace_purge(workspace_id);
    state.mcp.publish_resource_uris(uris).await;
    Ok(Json(result))
}

pub async fn erase_workspace(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<crate::dto::EraseWorkspace>,
) -> ApiResult<Json<WorkspaceEraseResult>> {
    let workspace_id = WorkspaceId(id);
    cap(&auth, WORKSPACE_WRITE)?;
    ensure_workspace(&auth, workspace_id)?;
    if body.confirm_workspace_id != workspace_id.0 {
        return Err(ApiError::BadRequest(
            "confirm_workspace_id must match path workspace id".into(),
        ));
    }
    state.store.get_workspace(workspace_id).await?;
    state
        .store
        .append_audit(NewAuditEvent {
            actor_id: Some(auth.member_id),
            action: "workspace.erase".into(),
            target_kind: Some("workspace".into()),
            target_id: Some(workspace_id.0),
            metadata: serde_json::json!({ "phase": "started" }),
        })
        .await?;
    let mut result = state.store.erase_workspace(workspace_id).await?;
    let mut artifact_blobs_deleted = 0u64;
    for sha_hex in &result.purge.artifact_shas {
        let Ok(sha) = maidan_artifacts::Sha256::from_hex(sha_hex) else {
            continue;
        };
        if state.artifacts.delete(&sha).await.is_ok() {
            artifact_blobs_deleted += 1;
        }
    }
    let _ = artifact_blobs_deleted;
    result.purge.artifact_shas.clear();
    let uris = maidan_mcp::resource_updates::uris_for_workspace_purge(workspace_id);
    state.mcp.publish_resource_uris(uris).await;
    Ok(Json(result))
}

pub async fn list_workspace_audit(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(workspace_id): Path<uuid::Uuid>,
    Query(q): Query<ListAuditQuery>,
) -> ApiResult<Json<Vec<AuditEvent>>> {
    let workspace_id = WorkspaceId(workspace_id);
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, workspace_id)?;
    let limit = q.limit.clamp(1, 500);
    Ok(Json(
        state
            .store
            .list_audit_for_workspace(workspace_id, limit)
            .await?,
    ))
}

pub async fn list_events(
    State(state): State<AppState>,
    Path(workspace_id): Path<uuid::Uuid>,
    Query(q): Query<ListEventsQuery>,
    auth: Option<Extension<AuthContext>>,
    peer: Option<Extension<PeerContext>>,
) -> ApiResult<Json<Vec<StoredEvent>>> {
    let workspace_id = WorkspaceId(workspace_id);
    match (&auth, &peer) {
        (Some(Extension(auth)), None) => {
            cap(auth, WORKSPACE_READ)?;
            ensure_workspace(auth, workspace_id)?;
        }
        (None, Some(Extension(PeerContext(peer)))) => {
            if peer.remote_workspace_id != workspace_id {
                return Err(ApiError::Forbidden(
                    "peer may only read its registered remote workspace".into(),
                ));
            }
        }
        _ => return Err(ApiError::Unauthorized),
    }
    Ok(Json(
        state
            .store
            .list_events_after(workspace_id, q.after_id, q.limit)
            .await?,
    ))
}

// --- members ---

pub async fn create_member(
    State(state): State<AppState>,
    Path(workspace_id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<CreateMember>,
) -> ApiResult<(StatusCode, Json<Member>)> {
    let m = state
        .store
        .create_member(NewMember {
            workspace_id: WorkspaceId(workspace_id),
            handle: body.handle,
            display_name: body.display_name,
            kind: body.kind,
        })
        .await?;
    publish(
        &state,
        Event::MemberJoined {
            occurred_at: Utc::now(),
            workspace_id: WorkspaceId(workspace_id),
            member: m.clone(),
        },
    )
    .await;
    Ok((StatusCode::CREATED, Json(m)))
}

pub async fn list_members(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(workspace_id): Path<uuid::Uuid>,
) -> ApiResult<Json<Vec<Member>>> {
    let workspace_id = WorkspaceId(workspace_id);
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, workspace_id)?;
    Ok(Json(state.store.list_members(workspace_id).await?))
}

pub async fn get_member(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<Member>> {
    let member = state.store.get_member(MemberId(id)).await?;
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, member.workspace_id)?;
    Ok(Json(member))
}

pub async fn list_mentions_for_member(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
    Query(q): Query<ListMentionsQuery>,
) -> ApiResult<Json<Vec<Mention>>> {
    let member = state.store.get_member(MemberId(id)).await?;
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, member.workspace_id)?;
    Ok(Json(
        state
            .store
            .list_mentions_for_member(MemberId(id), q.limit)
            .await?,
    ))
}

pub async fn get_member_inbox(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
    Query(q): Query<ListInboxQuery>,
) -> ApiResult<Json<MemberInbox>> {
    let member = state.store.get_member(MemberId(id)).await?;
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, member.workspace_id)?;
    Ok(Json(
        state.store.list_member_inbox(MemberId(id), q.limit).await?,
    ))
}

pub async fn mark_member_inbox_read(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<MarkInboxRead>,
) -> ApiResult<Json<MemberInbox>> {
    let member = state.store.get_member(MemberId(id)).await?;
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, member.workspace_id)?;
    state
        .store
        .advance_inbox_last_read_at(MemberId(id), body.read_through)
        .await?;
    Ok(Json(state.store.list_member_inbox(MemberId(id), 50).await?))
}

// --- channels ---

pub async fn create_channel(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(workspace_id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<CreateChannel>,
) -> ApiResult<(StatusCode, Json<Channel>)> {
    let workspace_id = WorkspaceId(workspace_id);
    cap(&auth, WORKSPACE_WRITE)?;
    ensure_workspace(&auth, workspace_id)?;
    let c = state
        .store
        .create_channel(NewChannel {
            workspace_id,
            name: body.name,
            topic: body.topic,
            private: body.private,
        })
        .await?;
    publish(
        &state,
        Event::ChannelCreated {
            occurred_at: Utc::now(),
            workspace_id,
            channel: c.clone(),
        },
    )
    .await;
    Ok((StatusCode::CREATED, Json(c)))
}

pub async fn list_channels(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(workspace_id): Path<uuid::Uuid>,
) -> ApiResult<Json<Vec<Channel>>> {
    let workspace_id = WorkspaceId(workspace_id);
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, workspace_id)?;
    Ok(Json(state.store.list_channels(workspace_id).await?))
}

pub async fn get_channel(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<Channel>> {
    let channel = state.store.get_channel(ChannelId(id)).await?;
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, channel.workspace_id)?;
    Ok(Json(channel))
}

// --- threads ---

pub async fn create_thread(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(channel_id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<CreateThread>,
) -> ApiResult<(StatusCode, Json<Thread>)> {
    let ctx = resolve_channel_context(state.store.as_ref(), ChannelId(channel_id)).await?;
    cap(&auth, WORKSPACE_WRITE)?;
    ensure_workspace(&auth, ctx.workspace_id)?;
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

// --- messages ---

pub async fn post_message(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(thread_id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<CreateMessage>,
) -> ApiResult<(StatusCode, Json<Message>)> {
    cap(&auth, MESSAGE_POST)?;
    let ctx = resolve_thread_context(state.store.as_ref(), ThreadId(thread_id)).await?;
    ensure_workspace(&auth, ctx.workspace_id)?;
    let parsed_slash = maidan_router::parse_slash_command(&body.body);
    let m = state
        .store
        .post_message(NewMessage {
            thread_id: ThreadId(thread_id),
            author_id: MemberId(body.author_id),
            body: body.body,
            metadata: body.metadata,
        })
        .await?;
    let mut message = m;
    if let Some(ref parsed) = parsed_slash {
        if state
            .store
            .get_slash_command_by_name(ctx.workspace_id, &parsed.name)
            .await
            .is_ok()
        {
            let slash_result = crate::slash_commands::dispatch_slash_command(
                &state,
                &auth,
                parsed,
                ctx.workspace_id,
                ctx.channel_id,
                ThreadId(thread_id),
                MemberId(body.author_id),
                message.id,
            )
            .await;
            let metadata = crate::slash_commands::merge_metadata(
                message.metadata.clone(),
                crate::slash_commands::slash_metadata(parsed, &slash_result),
            );
            message = state
                .store
                .edit_message(
                    message.id,
                    MemberId(body.author_id),
                    EditMessage {
                        body: message.body.clone(),
                        metadata,
                    },
                )
                .await?;
        }
    }
    publish(
        &state,
        Event::MessagePosted {
            occurred_at: Utc::now(),
            workspace_id: ctx.workspace_id,
            channel_id: ctx.channel_id,
            thread_id: ThreadId(thread_id),
            dm_conversation_id: state
                .store
                .dm_conversation_for_thread(ThreadId(thread_id))
                .await
                .ok()
                .flatten()
                .map(|d| d.id),
            message: message.clone(),
        },
    )
    .await;
    publish_routed_mentions(&state, ctx.thread_id, ctx.workspace_id, &message).await;
    Ok((StatusCode::CREATED, Json(message)))
}

pub async fn edit_message(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(message_id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<EditMessageRequest>,
) -> ApiResult<Json<Message>> {
    let message_id = MessageId(message_id);
    let chain = resolve_message_chain(state.store.as_ref(), message_id).await?;
    ensure_workspace(&auth, chain.workspace_id)?;
    let existing = state.store.get_message(message_id).await?;
    if existing.tombstoned_at.is_some() {
        return Err(ApiError::BadRequest("message is tombstoned".into()));
    }
    let editor_id = MemberId(body.editor_id);
    ensure_message_edit(&auth, editor_id, existing.author_id)?;
    let metadata = body.metadata.unwrap_or(existing.metadata);
    let updated = state
        .store
        .edit_message(
            message_id,
            editor_id,
            EditMessage {
                body: body.body,
                metadata,
            },
        )
        .await?;
    publish(
        &state,
        Event::MessageEdited {
            occurred_at: Utc::now(),
            workspace_id: chain.workspace_id,
            channel_id: chain.channel_id,
            thread_id: chain.thread_id,
            dm_conversation_id: crate::dm::dm_conversation_id_for_thread(
                state.store.as_ref(),
                chain.thread_id,
            )
            .await,
            editor_id,
            message: updated.clone(),
        },
    )
    .await;
    let uris =
        maidan_mcp::resource_updates::uris_for_message(state.store.as_ref(), message_id).await;
    state.mcp.publish_resource_uris(uris).await;
    Ok(Json(updated))
}

pub async fn list_messages(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(thread_id): Path<uuid::Uuid>,
    Query(q): Query<ListMessagesQuery>,
) -> ApiResult<Json<Vec<Message>>> {
    let ctx = resolve_thread_context(state.store.as_ref(), ThreadId(thread_id)).await?;
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, ctx.workspace_id)?;
    Ok(Json(
        state
            .store
            .list_messages(ThreadId(thread_id), q.limit)
            .await?,
    ))
}

pub async fn get_message(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<Message>> {
    let chain = resolve_message_chain(state.store.as_ref(), MessageId(id)).await?;
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, chain.workspace_id)?;
    Ok(Json(state.store.get_message(MessageId(id)).await?))
}

pub async fn list_message_edits(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
    Query(q): Query<crate::dto::ListMessageEditsQuery>,
) -> ApiResult<Json<Vec<MessageEdit>>> {
    let message_id = MessageId(id);
    let chain = resolve_message_chain(state.store.as_ref(), message_id).await?;
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, chain.workspace_id)?;
    let limit = q.limit.clamp(1, 500);
    Ok(Json(
        state.store.list_message_edits(message_id, limit).await?,
    ))
}

pub async fn tombstone_message(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<StatusCode> {
    let chain = resolve_message_chain(state.store.as_ref(), MessageId(id)).await?;
    cap(&auth, WORKSPACE_WRITE)?;
    ensure_workspace(&auth, chain.workspace_id)?;
    state.store.tombstone_message(MessageId(id)).await?;
    publish(
        &state,
        Event::MessageTombstoned {
            occurred_at: Utc::now(),
            workspace_id: chain.workspace_id,
            channel_id: chain.channel_id,
            thread_id: chain.thread_id,
            dm_conversation_id: crate::dm::dm_conversation_id_for_thread(
                state.store.as_ref(),
                chain.thread_id,
            )
            .await,
            message_id: MessageId(id),
        },
    )
    .await;
    let uris = maidan_mcp::resource_updates::uris_for_message_tombstone(
        state.store.as_ref(),
        MessageId(id),
    )
    .await;
    state.mcp.publish_resource_uris(uris).await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn purge_message(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<StatusCode> {
    let chain = resolve_message_chain(state.store.as_ref(), MessageId(id)).await?;
    cap(&auth, WORKSPACE_WRITE)?;
    ensure_workspace(&auth, chain.workspace_id)?;
    state.store.purge_message(MessageId(id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn create_mention(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(message_id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<CreateMention>,
) -> ApiResult<StatusCode> {
    let chain = resolve_message_chain(state.store.as_ref(), MessageId(message_id)).await?;
    cap(&auth, WORKSPACE_WRITE)?;
    ensure_workspace(&auth, chain.workspace_id)?;
    state
        .store
        .record_mention(MessageId(message_id), MemberId(body.member_id))
        .await?;
    publish(
        &state,
        Event::MentionRecorded {
            occurred_at: Utc::now(),
            workspace_id: chain.workspace_id,
            thread_id: chain.thread_id,
            message_id: MessageId(message_id),
            member_id: MemberId(body.member_id),
        },
    )
    .await;
    let uris =
        maidan_mcp::resource_updates::uris_for_message(state.store.as_ref(), MessageId(message_id))
            .await;
    state.mcp.publish_resource_uris(uris).await;
    Ok(StatusCode::NO_CONTENT)
}

// --- votes ---

pub async fn cast_vote(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(message_id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<CreateVote>,
) -> ApiResult<StatusCode> {
    let chain = resolve_message_chain(state.store.as_ref(), MessageId(message_id)).await?;
    cap(&auth, WORKSPACE_WRITE)?;
    ensure_workspace(&auth, chain.workspace_id)?;
    state
        .store
        .cast_vote(NewVote {
            message_id: MessageId(message_id),
            member_id: MemberId(body.member_id),
            kind: body.kind.clone(),
        })
        .await?;
    publish(
        &state,
        Event::VoteCast {
            occurred_at: Utc::now(),
            workspace_id: chain.workspace_id,
            thread_id: chain.thread_id,
            message_id: MessageId(message_id),
            member_id: MemberId(body.member_id),
            vote_kind: body.kind,
        },
    )
    .await;
    let uris =
        maidan_mcp::resource_updates::uris_for_message(state.store.as_ref(), MessageId(message_id))
            .await;
    state.mcp.publish_resource_uris(uris).await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_votes(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(message_id): Path<uuid::Uuid>,
) -> ApiResult<Json<Vec<Vote>>> {
    let chain = resolve_message_chain(state.store.as_ref(), MessageId(message_id)).await?;
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, chain.workspace_id)?;
    Ok(Json(
        state
            .store
            .list_votes_for_message(MessageId(message_id))
            .await?,
    ))
}

// --- reactions ---

pub async fn add_reaction(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(message_id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<CreateReaction>,
) -> ApiResult<StatusCode> {
    let message_id = MessageId(message_id);
    let chain = resolve_message_chain(state.store.as_ref(), message_id).await?;
    cap(&auth, WORKSPACE_WRITE)?;
    ensure_workspace(&auth, chain.workspace_id)?;
    let member_id = MemberId(body.member_id);
    let emoji = body.emoji.clone();
    state
        .store
        .add_reaction(NewReaction {
            message_id,
            member_id,
            emoji: emoji.clone(),
        })
        .await?;
    publish(
        &state,
        Event::ReactionAdded {
            occurred_at: Utc::now(),
            workspace_id: chain.workspace_id,
            thread_id: chain.thread_id,
            message_id,
            member_id,
            emoji,
        },
    )
    .await;
    let uris =
        maidan_mcp::resource_updates::uris_for_message(state.store.as_ref(), message_id).await;
    state.mcp.publish_resource_uris(uris).await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn remove_reaction(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(message_id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<RemoveReaction>,
) -> ApiResult<StatusCode> {
    let message_id = MessageId(message_id);
    let chain = resolve_message_chain(state.store.as_ref(), message_id).await?;
    cap(&auth, WORKSPACE_WRITE)?;
    ensure_workspace(&auth, chain.workspace_id)?;
    let member_id = MemberId(body.member_id);
    let emoji = body.emoji.clone();
    if state
        .store
        .remove_reaction(message_id, member_id, &emoji)
        .await?
    {
        publish(
            &state,
            Event::ReactionRemoved {
                occurred_at: Utc::now(),
                workspace_id: chain.workspace_id,
                thread_id: chain.thread_id,
                message_id,
                member_id,
                emoji,
            },
        )
        .await;
        let uris =
            maidan_mcp::resource_updates::uris_for_message(state.store.as_ref(), message_id).await;
        state.mcp.publish_resource_uris(uris).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_reactions(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(message_id): Path<uuid::Uuid>,
) -> ApiResult<Json<Vec<Reaction>>> {
    let chain = resolve_message_chain(state.store.as_ref(), MessageId(message_id)).await?;
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, chain.workspace_id)?;
    Ok(Json(
        state
            .store
            .list_reactions_for_message(MessageId(message_id))
            .await?,
    ))
}

// --- pins ---

pub async fn pin_message(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(thread_id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<PinMessage>,
) -> ApiResult<StatusCode> {
    let thread_id = ThreadId(thread_id);
    let ctx = resolve_thread_context(state.store.as_ref(), thread_id).await?;
    cap(&auth, WORKSPACE_WRITE)?;
    ensure_workspace(&auth, ctx.workspace_id)?;
    let message_id = MessageId(body.message_id);
    let member_id = MemberId(body.member_id);
    state
        .store
        .pin_message(NewPin {
            thread_id,
            message_id,
            member_id,
        })
        .await?;
    publish(
        &state,
        Event::MessagePinned {
            occurred_at: Utc::now(),
            workspace_id: ctx.workspace_id,
            channel_id: ctx.channel_id,
            thread_id,
            message_id,
            member_id,
        },
    )
    .await;
    let uris =
        maidan_mcp::resource_updates::uris_for_thread_transition(state.store.as_ref(), thread_id)
            .await;
    state.mcp.publish_resource_uris(uris).await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn unpin_message(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(thread_id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<PinMessage>,
) -> ApiResult<StatusCode> {
    let thread_id = ThreadId(thread_id);
    let ctx = resolve_thread_context(state.store.as_ref(), thread_id).await?;
    cap(&auth, WORKSPACE_WRITE)?;
    ensure_workspace(&auth, ctx.workspace_id)?;
    let message_id = MessageId(body.message_id);
    let member_id = MemberId(body.member_id);
    if state.store.unpin_message(thread_id, message_id).await? {
        publish(
            &state,
            Event::MessageUnpinned {
                occurred_at: Utc::now(),
                workspace_id: ctx.workspace_id,
                channel_id: ctx.channel_id,
                thread_id,
                message_id,
                member_id,
            },
        )
        .await;
        let uris = maidan_mcp::resource_updates::uris_for_thread_transition(
            state.store.as_ref(),
            thread_id,
        )
        .await;
        state.mcp.publish_resource_uris(uris).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_pins(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(thread_id): Path<uuid::Uuid>,
) -> ApiResult<Json<Vec<Pin>>> {
    let ctx = resolve_thread_context(state.store.as_ref(), ThreadId(thread_id)).await?;
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, ctx.workspace_id)?;
    Ok(Json(
        state
            .store
            .list_pins_for_thread(ThreadId(thread_id))
            .await?,
    ))
}

// --- artifacts ---

fn s3_artifacts(artifacts: &Arc<dyn ArtifactStore>) -> Result<&S3Store, ApiError> {
    artifacts
        .as_ref()
        .as_any()
        .downcast_ref::<S3Store>()
        .ok_or_else(|| {
            ApiError::BadRequest(
                "multipart uploads require S3 artifact backend (ARTIFACT_BACKEND=s3)".into(),
            )
        })
}

fn multipart_upload(upload_id: &str, object_key: &str) -> MultipartUpload {
    MultipartUpload {
        upload_id: upload_id.to_string(),
        object_key: object_key.to_string(),
    }
}

pub async fn begin_multipart_artifact(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
) -> ApiResult<(StatusCode, Json<MultipartUploadResponse>)> {
    cap(&auth, ARTIFACT_UPLOAD)?;
    let upload = s3_artifacts(&state.artifacts)?
        .begin_multipart_upload()
        .await?;
    Ok((
        StatusCode::CREATED,
        Json(MultipartUploadResponse {
            upload_id: upload.upload_id,
            object_key: upload.object_key,
        }),
    ))
}

pub async fn upload_multipart_artifact_part(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path((upload_id, part_number)): Path<(String, i32)>,
    Query(q): Query<MultipartUploadQuery>,
    body: Bytes,
) -> ApiResult<Json<MultipartPartResponse>> {
    cap(&auth, ARTIFACT_UPLOAD)?;
    if body.is_empty() {
        return Err(ApiError::BadRequest("empty part body".into()));
    }
    let upload = multipart_upload(&upload_id, &q.object_key);
    let etag = s3_artifacts(&state.artifacts)?
        .upload_part(&upload, part_number, body)
        .await?;
    Ok(Json(MultipartPartResponse { part_number, etag }))
}

pub async fn complete_multipart_artifact(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(upload_id): Path<String>,
    ApiJson(body): ApiJson<CompleteMultipartArtifact>,
) -> ApiResult<(StatusCode, Json<Artifact>)> {
    cap(&auth, ARTIFACT_UPLOAD)?;
    let upload = multipart_upload(&upload_id, &body.object_key);
    let parts: Vec<CompletedPart> = body
        .parts
        .into_iter()
        .map(|p| CompletedPart {
            part_number: p.part_number,
            etag: p.etag,
        })
        .collect();
    let sha = s3_artifacts(&state.artifacts)?
        .complete_multipart_upload(&upload, &parts)
        .await?;
    let bytes = state.artifacts.get(&sha).await?;
    let artifact = state
        .store
        .upsert_artifact(NewArtifact {
            sha256: sha.to_string(),
            size_bytes: bytes.len() as i64,
            mime_type: body.mime_type,
            kind: body.kind,
            uploaded_by: body.uploaded_by.map(MemberId),
        })
        .await?;
    publish(
        &state,
        Event::ArtifactUpserted {
            occurred_at: Utc::now(),
            artifact: artifact.clone(),
        },
    )
    .await;
    Ok((StatusCode::CREATED, Json(artifact)))
}

pub async fn abort_multipart_artifact(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Query(q): Query<AbortMultipartQuery>,
) -> ApiResult<StatusCode> {
    cap(&auth, ARTIFACT_UPLOAD)?;
    let upload = multipart_upload(&q.upload_id, &q.object_key);
    s3_artifacts(&state.artifacts)?
        .abort_multipart_upload(&upload)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn upload_artifact(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Query(q): Query<UploadArtifactQuery>,
    body: Bytes,
) -> ApiResult<(StatusCode, Json<Artifact>)> {
    cap(&auth, ARTIFACT_UPLOAD)?;
    if body.is_empty() {
        return Err(ApiError::BadRequest("empty artifact body".into()));
    }
    let sha = state.artifacts.put(body.clone()).await?;
    let artifact = state
        .store
        .upsert_artifact(NewArtifact {
            sha256: sha.to_string(),
            size_bytes: body.len() as i64,
            mime_type: q.mime_type,
            kind: q.kind,
            uploaded_by: q.uploaded_by.map(MemberId),
        })
        .await?;
    publish(
        &state,
        Event::ArtifactUpserted {
            occurred_at: Utc::now(),
            artifact: artifact.clone(),
        },
    )
    .await;
    Ok((StatusCode::CREATED, Json(artifact)))
}

pub async fn get_artifact(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(sha_hex): Path<String>,
) -> ApiResult<Response> {
    cap(&auth, WORKSPACE_READ)?;
    let sha = Sha256::from_hex(&sha_hex).map_err(|e| ApiError::BadRequest(e.to_string()))?;
    let meta = state.store.get_artifact_by_sha(&sha_hex).await?;
    let bytes = state.artifacts.get(&sha).await?;
    let mut headers = HeaderMap::new();
    if let Some(mime) = meta.mime_type {
        if let Ok(value) = mime.parse() {
            headers.insert(header::CONTENT_TYPE, value);
        }
    }
    headers.insert(
        header::HeaderName::from_static("x-artifact-kind"),
        meta.kind.as_str().parse().unwrap(),
    );
    Ok((headers, bytes).into_response())
}

pub async fn get_artifact_metadata(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(sha_hex): Path<String>,
) -> ApiResult<Json<Artifact>> {
    cap(&auth, WORKSPACE_READ)?;
    Sha256::from_hex(&sha_hex).map_err(|e| ApiError::BadRequest(e.to_string()))?;
    Ok(Json(state.store.get_artifact_by_sha(&sha_hex).await?))
}

// --- references ---

pub async fn create_reference(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiJson(body): ApiJson<CreateReference>,
) -> ApiResult<(StatusCode, Json<Reference>)> {
    cap(&auth, WORKSPACE_WRITE)?;
    let r = state
        .store
        .add_reference(NewReference {
            src_kind: body.src_kind,
            src_id: body.src_id,
            dst_kind: body.dst_kind,
            dst_id: body.dst_id,
            relation: body.relation,
        })
        .await?;
    publish(
        &state,
        Event::ReferenceAdded {
            occurred_at: Utc::now(),
            reference: r.clone(),
        },
    )
    .await;
    Ok((StatusCode::CREATED, Json(r)))
}

pub async fn list_references(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Query(q): Query<ListReferencesQuery>,
) -> ApiResult<Json<Vec<Reference>>> {
    cap(&auth, WORKSPACE_READ)?;
    Ok(Json(
        state
            .store
            .list_references_from(q.src_kind, q.src_id)
            .await?,
    ))
}

// --- search ---

pub async fn search_messages(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(workspace_id): Path<uuid::Uuid>,
    Query(q): Query<SearchQuery>,
) -> ApiResult<Json<Vec<maidan_search::SearchHit>>> {
    let workspace_id = WorkspaceId(workspace_id);
    cap(&auth, SEARCH_QUERY)?;
    ensure_workspace(&auth, workspace_id)?;
    let filters = maidan_search::SearchFilters {
        author_id: q.author.map(MemberId),
        channel_id: q.channel.map(ChannelId),
        author_kind: q.kind,
    };
    let hits = match q.mode {
        SearchMode::Lexical => {
            state
                .search
                .search_messages(workspace_id, &q.q, q.limit, &filters)
                .await?
        }
        SearchMode::Semantic => {
            let embedding = state
                .embedding_provider
                .embed(&q.q)
                .map_err(|e| ApiError::Internal(format!("embedding generation failed: {e}")))?;
            let model = q
                .embedding_model
                .as_deref()
                .unwrap_or_else(|| state.embedding_provider.model_name());
            state
                .search
                .semantic_search(workspace_id, &embedding, q.limit, &filters, model)
                .await?
        }
    };
    Ok(Json(hits))
}

// --- api tokens ---

pub async fn mint_api_token(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path((workspace_id, member_id)): Path<(uuid::Uuid, uuid::Uuid)>,
    ApiJson(body): ApiJson<MintApiToken>,
) -> ApiResult<(StatusCode, Json<MintApiTokenResponse>)> {
    let workspace_id = WorkspaceId(workspace_id);
    let member_id = MemberId(member_id);
    cap(&auth, TOKEN_ADMIN)?;
    ensure_workspace(&auth, workspace_id)?;

    let member = state.store.get_member(member_id).await?;
    if member.workspace_id != workspace_id {
        return Err(ApiError::BadRequest(
            "member does not belong to workspace".into(),
        ));
    }

    let capabilities = if body.capabilities.is_empty() {
        capability::default_minted()
    } else {
        capability::validate_list(&body.capabilities).map_err(ApiError::BadRequest)?;
        body.capabilities
    };
    crate::quota::validate_token_quotas(&body.quotas, &capabilities)?;

    let secret = TokenSecret::generate();
    let record = state
        .store
        .create_api_token(NewApiToken {
            workspace_id,
            member_id,
            app_installation_id: None,
            token_hash: hash_secret(secret.as_str()),
            label: body.label,
            capabilities: capabilities.clone(),
            expires_at: body.expires_at,
        })
        .await?;

    if !body.quotas.is_empty() {
        state
            .store
            .replace_token_quotas(record.id, &body.quotas)
            .await?;
    }
    let quotas = state.store.list_token_quotas(record.id).await?;

    Ok((
        StatusCode::CREATED,
        Json(MintApiTokenResponse {
            id: record.id,
            secret: secret.as_str().to_string(),
            workspace_id: record.workspace_id,
            member_id: record.member_id,
            capabilities: record.capabilities,
            expires_at: record.expires_at,
            quotas,
        }),
    ))
}

pub async fn revoke_api_token(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<ApiToken>> {
    cap(&auth, TOKEN_ADMIN)?;
    let token_id = ApiTokenId(id);
    let existing = state.store.get_api_token(token_id).await?;
    ensure_workspace(&auth, existing.workspace_id)?;
    Ok(Json(state.store.revoke_api_token(token_id).await?))
}

pub(crate) async fn publish_routed_mentions(
    state: &AppState,
    thread_id: ThreadId,
    workspace_id: WorkspaceId,
    message: &Message,
) {
    let mentioned = match route_mentions_for_message(
        state.store.as_ref(),
        message.id,
        message.author_id,
        &message.body,
    )
    .await
    {
        Ok(ids) => ids,
        Err(err) => {
            tracing::warn!(error = %err, "mention routing failed");
            return;
        }
    };
    for member_id in mentioned {
        publish(
            state,
            Event::MentionRecorded {
                occurred_at: Utc::now(),
                workspace_id,
                thread_id,
                message_id: message.id,
                member_id,
            },
        )
        .await;
    }
}

/// Fire-and-forget event publish. Errors are logged but never surfaced
/// to the HTTP caller — the store has already committed, and the bus
/// being temporarily unavailable should not turn a successful mutation
/// into a 5xx.
///
/// Returns the new `log_id` when append succeeded.
pub(crate) async fn publish(state: &AppState, event: Event) -> Option<i64> {
    let stored = match state.store.append_event(&event).await {
        Ok(row) => row,
        Err(err) => {
            tracing::warn!(error = %err, "event log append failed");
            return None;
        }
    };
    if state.outbox_relay {
        return Some(stored.id);
    }
    let envelope = BusEnvelope {
        log_id: stored.id,
        event,
    };
    if let Err(err) = state.bus.publish(envelope).await {
        tracing::warn!(error = %err, "bus publish failed");
    }
    Some(stored.id)
}

#[cfg(test)]
mod publish_tests {
    use std::sync::Arc;

    use chrono::Utc;
    use maidan_artifacts::LocalFsStore;
    use maidan_bus::{test_support::RecordingBus, InMemoryBus};
    use maidan_search::SqliteSearch;
    use maidan_store::{run_sqlite_migrations, SqliteStore};
    use maidan_types::*;
    use sqlx::sqlite::SqlitePoolOptions;

    use super::publish;
    use crate::state::AppState;

    async fn sqlite_state(bus: Arc<dyn maidan_bus::EventBus>) -> AppState {
        let pool = SqlitePoolOptions::new()
            .connect("sqlite::memory:")
            .await
            .expect("connect");
        sqlx::query("PRAGMA foreign_keys = ON")
            .execute(&pool)
            .await
            .expect("fk");
        run_sqlite_migrations(&pool).await.expect("migrate");
        let store = Arc::new(SqliteStore::new(pool.clone()));
        let search: Arc<dyn maidan_search::Search> = Arc::new(SqliteSearch::new(pool));
        let artifacts = Arc::new(LocalFsStore::new(tempfile::tempdir().unwrap().path()));
        AppState::for_tests(store, artifacts, bus, search)
    }

    #[tokio::test]
    async fn publish_calls_bus_when_outbox_relay_disabled() {
        let inner = Arc::new(InMemoryBus::new());
        let bus = Arc::new(RecordingBus::new(inner));
        let mut state = sqlite_state(bus.clone()).await;
        state.outbox_relay = false;

        let event = Event::WorkspaceCreated {
            occurred_at: Utc::now(),
            workspace: Workspace {
                id: WorkspaceId(uuid::Uuid::new_v4()),
                name: "pub-ws".into(),
                created_at: Utc::now(),
                updated_at: Utc::now(),
                tombstoned_at: None,
            },
        };
        let log_id = publish(&state, event).await;
        assert!(log_id.is_some());
        assert_eq!(bus.publishes(), 1);
    }

    #[tokio::test]
    async fn publish_skips_bus_when_outbox_relay_enabled() {
        let inner = Arc::new(InMemoryBus::new());
        let bus = Arc::new(RecordingBus::new(inner));
        let mut state = sqlite_state(bus.clone()).await;
        state.outbox_relay = true;

        let event = Event::WorkspaceCreated {
            occurred_at: Utc::now(),
            workspace: Workspace {
                id: WorkspaceId(uuid::Uuid::new_v4()),
                name: "defer-ws".into(),
                created_at: Utc::now(),
                updated_at: Utc::now(),
                tombstoned_at: None,
            },
        };
        let log_id = publish(&state, event).await;
        assert!(log_id.is_some());
        assert_eq!(bus.publishes(), 0);
    }
}

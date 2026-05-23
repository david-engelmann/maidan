//! HTTP CRUD handlers. Every handler returns `Result<Json<_>, ApiError>`
//! and lets the [`crate::error::ApiError`] type render the failure as
//! `application/problem+json`. Mutations publish an [`Event`] to the
//! bus after the store call succeeds.

use axum::{
    body::Bytes,
    extract::{Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use chrono::Utc;
use maidan_artifacts::Sha256;
use maidan_fsm::ThreadAction;
use maidan_store::Store;
use maidan_types::*;

use crate::dto::*;
use crate::error::{ApiError, ApiJson};
use crate::state::AppState;

type ApiResult<T> = Result<T, ApiError>;

async fn workspace_for_thread(
    store: &dyn Store,
    thread_id: ThreadId,
) -> Result<(WorkspaceId, ChannelId), ApiError> {
    let thread = store.get_thread(thread_id).await?;
    let channel = store.get_channel(thread.channel_id).await?;
    Ok((channel.workspace_id, channel.id))
}

async fn chain_for_message(
    store: &dyn Store,
    message_id: MessageId,
) -> Result<(WorkspaceId, ChannelId, ThreadId), ApiError> {
    let message = store.get_message(message_id).await?;
    let (workspace_id, channel_id) = workspace_for_thread(store, message.thread_id).await?;
    Ok((workspace_id, channel_id, message.thread_id))
}

// --- workspaces ---

pub async fn create_workspace(
    State(state): State<AppState>,
    ApiJson(body): ApiJson<CreateWorkspace>,
) -> ApiResult<(StatusCode, Json<Workspace>)> {
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
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<Workspace>> {
    Ok(Json(state.store.get_workspace(WorkspaceId(id)).await?))
}

pub async fn list_events(
    State(state): State<AppState>,
    Path(workspace_id): Path<uuid::Uuid>,
    Query(q): Query<ListEventsQuery>,
) -> ApiResult<Json<Vec<StoredEvent>>> {
    Ok(Json(
        state
            .store
            .list_events_after(WorkspaceId(workspace_id), q.after_id, q.limit)
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
    Path(workspace_id): Path<uuid::Uuid>,
) -> ApiResult<Json<Vec<Member>>> {
    Ok(Json(
        state.store.list_members(WorkspaceId(workspace_id)).await?,
    ))
}

pub async fn get_member(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<Member>> {
    Ok(Json(state.store.get_member(MemberId(id)).await?))
}

pub async fn list_mentions_for_member(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
    Query(q): Query<ListMentionsQuery>,
) -> ApiResult<Json<Vec<Mention>>> {
    Ok(Json(
        state
            .store
            .list_mentions_for_member(MemberId(id), q.limit)
            .await?,
    ))
}

// --- channels ---

pub async fn create_channel(
    State(state): State<AppState>,
    Path(workspace_id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<CreateChannel>,
) -> ApiResult<(StatusCode, Json<Channel>)> {
    let c = state
        .store
        .create_channel(NewChannel {
            workspace_id: WorkspaceId(workspace_id),
            name: body.name,
            topic: body.topic,
            private: body.private,
        })
        .await?;
    publish(
        &state,
        Event::ChannelCreated {
            occurred_at: Utc::now(),
            workspace_id: WorkspaceId(workspace_id),
            channel: c.clone(),
        },
    )
    .await;
    Ok((StatusCode::CREATED, Json(c)))
}

pub async fn list_channels(
    State(state): State<AppState>,
    Path(workspace_id): Path<uuid::Uuid>,
) -> ApiResult<Json<Vec<Channel>>> {
    Ok(Json(
        state.store.list_channels(WorkspaceId(workspace_id)).await?,
    ))
}

pub async fn get_channel(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<Channel>> {
    Ok(Json(state.store.get_channel(ChannelId(id)).await?))
}

// --- threads ---

pub async fn create_thread(
    State(state): State<AppState>,
    Path(channel_id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<CreateThread>,
) -> ApiResult<(StatusCode, Json<Thread>)> {
    let channel = state.store.get_channel(ChannelId(channel_id)).await?;
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
            workspace_id: channel.workspace_id,
            channel_id: ChannelId(channel_id),
            thread: t.clone(),
        },
    )
    .await;
    Ok((StatusCode::CREATED, Json(t)))
}

pub async fn list_threads(
    State(state): State<AppState>,
    Path(channel_id): Path<uuid::Uuid>,
) -> ApiResult<Json<Vec<Thread>>> {
    Ok(Json(state.store.list_threads(ChannelId(channel_id)).await?))
}

pub async fn get_thread(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<Thread>> {
    Ok(Json(state.store.get_thread(ThreadId(id)).await?))
}

pub async fn transition_thread(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<TransitionThread>,
) -> ApiResult<Json<Thread>> {
    let action = ThreadAction::parse(&body.action).ok_or_else(|| {
        ApiError::BadRequest(format!(
            "unknown action {:?}; expected start_review, close, or archive",
            body.action
        ))
    })?;
    let thread_id = ThreadId(id);
    let (workspace_id, channel_id) = workspace_for_thread(state.store.as_ref(), thread_id).await?;
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
    Ok(Json(result.thread))
}

// --- messages ---

pub async fn post_message(
    State(state): State<AppState>,
    Path(thread_id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<CreateMessage>,
) -> ApiResult<(StatusCode, Json<Message>)> {
    let (workspace_id, channel_id) =
        workspace_for_thread(state.store.as_ref(), ThreadId(thread_id)).await?;
    let m = state
        .store
        .post_message(NewMessage {
            thread_id: ThreadId(thread_id),
            author_id: MemberId(body.author_id),
            body: body.body,
            metadata: body.metadata,
        })
        .await?;
    publish(
        &state,
        Event::MessagePosted {
            occurred_at: Utc::now(),
            workspace_id,
            channel_id,
            thread_id: ThreadId(thread_id),
            message: m.clone(),
        },
    )
    .await;
    Ok((StatusCode::CREATED, Json(m)))
}

pub async fn list_messages(
    State(state): State<AppState>,
    Path(thread_id): Path<uuid::Uuid>,
    Query(q): Query<ListMessagesQuery>,
) -> ApiResult<Json<Vec<Message>>> {
    Ok(Json(
        state
            .store
            .list_messages(ThreadId(thread_id), q.limit)
            .await?,
    ))
}

pub async fn get_message(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<Message>> {
    Ok(Json(state.store.get_message(MessageId(id)).await?))
}

pub async fn tombstone_message(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<StatusCode> {
    let (workspace_id, channel_id, thread_id) =
        chain_for_message(state.store.as_ref(), MessageId(id)).await?;
    state.store.tombstone_message(MessageId(id)).await?;
    publish(
        &state,
        Event::MessageTombstoned {
            occurred_at: Utc::now(),
            workspace_id,
            channel_id,
            thread_id,
            message_id: MessageId(id),
        },
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn create_mention(
    State(state): State<AppState>,
    Path(message_id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<CreateMention>,
) -> ApiResult<StatusCode> {
    let (workspace_id, _channel_id, thread_id) =
        chain_for_message(state.store.as_ref(), MessageId(message_id)).await?;
    state
        .store
        .record_mention(MessageId(message_id), MemberId(body.member_id))
        .await?;
    publish(
        &state,
        Event::MentionRecorded {
            occurred_at: Utc::now(),
            workspace_id,
            thread_id,
            message_id: MessageId(message_id),
            member_id: MemberId(body.member_id),
        },
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

// --- votes ---

pub async fn cast_vote(
    State(state): State<AppState>,
    Path(message_id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<CreateVote>,
) -> ApiResult<StatusCode> {
    let (workspace_id, _channel_id, thread_id) =
        chain_for_message(state.store.as_ref(), MessageId(message_id)).await?;
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
            workspace_id,
            thread_id,
            message_id: MessageId(message_id),
            member_id: MemberId(body.member_id),
            vote_kind: body.kind,
        },
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn list_votes(
    State(state): State<AppState>,
    Path(message_id): Path<uuid::Uuid>,
) -> ApiResult<Json<Vec<Vote>>> {
    Ok(Json(
        state
            .store
            .list_votes_for_message(MessageId(message_id))
            .await?,
    ))
}

// --- artifacts ---

pub async fn upload_artifact(
    State(state): State<AppState>,
    Query(q): Query<UploadArtifactQuery>,
    body: Bytes,
) -> ApiResult<(StatusCode, Json<Artifact>)> {
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
    Path(sha_hex): Path<String>,
) -> ApiResult<Response> {
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
    Path(sha_hex): Path<String>,
) -> ApiResult<Json<Artifact>> {
    Sha256::from_hex(&sha_hex).map_err(|e| ApiError::BadRequest(e.to_string()))?;
    Ok(Json(state.store.get_artifact_by_sha(&sha_hex).await?))
}

// --- references ---

pub async fn create_reference(
    State(state): State<AppState>,
    ApiJson(body): ApiJson<CreateReference>,
) -> ApiResult<(StatusCode, Json<Reference>)> {
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
    Query(q): Query<ListReferencesQuery>,
) -> ApiResult<Json<Vec<Reference>>> {
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
    Path(workspace_id): Path<uuid::Uuid>,
    Query(q): Query<SearchQuery>,
) -> ApiResult<Json<Vec<maidan_search::SearchHit>>> {
    Ok(Json(
        state
            .search
            .search_messages(WorkspaceId(workspace_id), &q.q, q.limit)
            .await?,
    ))
}

/// Fire-and-forget event publish. Errors are logged but never surfaced
/// to the HTTP caller — the store has already committed, and the bus
/// being temporarily unavailable should not turn a successful mutation
/// into a 5xx.
async fn publish(state: &AppState, event: Event) {
    if let Err(err) = state.store.append_event(&event).await {
        tracing::warn!(error = %err, "event log append failed");
    }
    if let Err(err) = state.bus.publish(event).await {
        tracing::warn!(error = %err, "bus publish failed");
    }
}

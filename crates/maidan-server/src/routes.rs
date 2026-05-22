//! HTTP CRUD handlers. Every handler returns `Result<Json<_>, ApiError>`
//! and lets the [`crate::error::ApiError`] type render the failure as
//! `application/problem+json`.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use maidan_types::*;

use crate::dto::*;
use crate::error::{ApiError, ApiJson};
use crate::state::AppState;

type ApiResult<T> = Result<T, ApiError>;

// --- workspaces ---

pub async fn create_workspace(
    State(state): State<AppState>,
    ApiJson(body): ApiJson<CreateWorkspace>,
) -> ApiResult<(StatusCode, Json<Workspace>)> {
    let ws = state
        .store
        .create_workspace(NewWorkspace { name: body.name })
        .await?;
    Ok((StatusCode::CREATED, Json(ws)))
}

pub async fn get_workspace(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<Workspace>> {
    Ok(Json(state.store.get_workspace(WorkspaceId(id)).await?))
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
    let t = state
        .store
        .create_thread(NewThread {
            channel_id: ChannelId(channel_id),
            title: body.title,
        })
        .await?;
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

// --- messages ---

pub async fn post_message(
    State(state): State<AppState>,
    Path(thread_id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<CreateMessage>,
) -> ApiResult<(StatusCode, Json<Message>)> {
    let m = state
        .store
        .post_message(NewMessage {
            thread_id: ThreadId(thread_id),
            author_id: MemberId(body.author_id),
            body: body.body,
            metadata: body.metadata,
        })
        .await?;
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
    state.store.tombstone_message(MessageId(id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn create_mention(
    State(state): State<AppState>,
    Path(message_id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<CreateMention>,
) -> ApiResult<StatusCode> {
    state
        .store
        .record_mention(MessageId(message_id), MemberId(body.member_id))
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

// --- votes ---

pub async fn cast_vote(
    State(state): State<AppState>,
    Path(message_id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<CreateVote>,
) -> ApiResult<StatusCode> {
    state
        .store
        .cast_vote(NewVote {
            message_id: MessageId(message_id),
            member_id: MemberId(body.member_id),
            kind: body.kind,
        })
        .await?;
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

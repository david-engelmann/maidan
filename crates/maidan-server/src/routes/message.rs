//! Message handlers: post/list/get messages, edits, tombstone/purge, and
//! mention recording.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Extension, Json,
};
use chrono::Utc;
use maidan_auth::{
    capability::{MESSAGE_POST, WORKSPACE_READ, WORKSPACE_WRITE},
    AuthContext,
};
use maidan_router::{resolve_message_chain, resolve_thread_context};
use maidan_types::*;

use super::{
    cap, ensure_message_edit, ensure_workspace, publish, publish_routed_mentions, ApiResult,
};
use crate::dto::*;
use crate::error::{ApiError, ApiJson};
use crate::state::AppState;

pub async fn post_message(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(thread_id): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<CreateMessage>,
) -> ApiResult<(StatusCode, Json<Message>)> {
    cap(&auth, MESSAGE_POST)?;
    let ctx = resolve_thread_context(state.store.as_ref(), ThreadId(thread_id)).await?;
    ensure_workspace(&auth, ctx.workspace_id)?;
    maidan_auth::ensure_thread_access(state.store.as_ref(), &auth, ThreadId(thread_id)).await?;
    super::ensure_acting_member(&auth, MemberId(body.author_id))?;
    // Cluster 173: a content-only post derives its searchable `body` from the
    // blocks; an explicit `body` is respected verbatim.
    let content = body.content.clone();
    let post_body = if body.body.is_empty() {
        content
            .as_deref()
            .map(maidan_types::derive_body)
            .unwrap_or_default()
    } else {
        body.body.clone()
    };
    let parsed_slash = maidan_router::parse_slash_command(&post_body);
    // A registered slash command will fire → the message gets edited after insert,
    // so its `MessagePosted` must reflect the post-edit message. Resolve this
    // before inserting so we can pick the atomic path.
    let slash_will_run = match &parsed_slash {
        Some(parsed) => state
            .store
            .get_slash_command_by_name(ctx.workspace_id, &parsed.name)
            .await
            .is_ok(),
        None => false,
    };
    let dm_conversation_id = state
        .store
        .dm_conversation_for_thread(ThreadId(thread_id))
        .await
        .ok()
        .flatten()
        .map(|d| d.id);
    let new_message = NewMessage {
        thread_id: ThreadId(thread_id),
        author_id: MemberId(body.author_id),
        body: post_body,
        metadata: body.metadata.clone(),
        content,
    };

    // Cluster 211: both paths commit the message and its `MessagePosted` event in
    // one transaction. No slash → insert + event atomically. Slash → provisional
    // insert, run the (possibly external) dispatch, then edit + event atomically
    // so the event carries the post-slash message.
    let message = if let Some(parsed) = parsed_slash.filter(|_| slash_will_run) {
        let m = state.store.post_message(new_message).await?;
        let slash_result = crate::slash_commands::dispatch_slash_command(
            &state,
            &auth,
            &parsed,
            ctx.workspace_id,
            ctx.channel_id,
            ThreadId(thread_id),
            MemberId(body.author_id),
            m.id,
        )
        .await;
        let metadata = crate::slash_commands::merge_metadata(
            m.metadata.clone(),
            crate::slash_commands::slash_metadata(&parsed, &slash_result),
        );
        let (message, stored) = state
            .store
            .edit_message_with_posted_event(
                m.id,
                MemberId(body.author_id),
                EditMessage {
                    body: m.body.clone(),
                    metadata,
                    content: m.content.clone(),
                },
                dm_conversation_id,
            )
            .await?;
        super::publish_stored(&state, stored).await;
        message
    } else {
        let (message, stored) = state
            .store
            .post_message_with_event(new_message, dm_conversation_id)
            .await?;
        super::publish_stored(&state, stored).await;
        message
    };
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
    maidan_auth::ensure_thread_access(state.store.as_ref(), &auth, chain.thread_id).await?;
    let existing = state.store.get_message(message_id).await?;
    if existing.tombstoned_at.is_some() {
        return Err(ApiError::BadRequest("message is tombstoned".into()));
    }
    let editor_id = MemberId(body.editor_id);
    super::ensure_acting_member(&auth, editor_id)?;
    ensure_message_edit(&auth, editor_id, existing.author_id)?;
    let metadata = body.metadata.unwrap_or(existing.metadata);
    // Cluster 173: omitted content keeps the existing blocks; a content edit
    // with an empty body re-derives the searchable body.
    let content = body.content.or(existing.content);
    let edit_body = if body.body.is_empty() {
        content
            .as_deref()
            .map(maidan_types::derive_body)
            .unwrap_or_default()
    } else {
        body.body
    };
    let updated = state
        .store
        .edit_message(
            message_id,
            editor_id,
            EditMessage {
                body: edit_body,
                metadata,
                content,
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
    maidan_auth::ensure_thread_access(state.store.as_ref(), &auth, ThreadId(thread_id)).await?;
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
    maidan_auth::ensure_thread_access(state.store.as_ref(), &auth, chain.thread_id).await?;
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
    maidan_auth::ensure_thread_access(state.store.as_ref(), &auth, chain.thread_id).await?;
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
    maidan_auth::ensure_thread_access(state.store.as_ref(), &auth, chain.thread_id).await?;
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
    maidan_auth::ensure_thread_access(state.store.as_ref(), &auth, chain.thread_id).await?;
    state.store.purge_message(MessageId(id)).await?;
    crate::audit::record(
        &state,
        NewAuditEvent {
            actor_id: Some(auth.member_id),
            action: "message.purge".into(),
            target_kind: Some("message".into()),
            target_id: Some(id),
            metadata: serde_json::json!({
                "workspace_id": chain.workspace_id.0,
                "channel_id": chain.channel_id.0,
                "thread_id": chain.thread_id.0,
            }),
        },
    )
    .await;
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
    maidan_auth::ensure_thread_access(state.store.as_ref(), &auth, chain.thread_id).await?;
    let stored = state
        .store
        .record_mention_with_event(MessageId(message_id), MemberId(body.member_id))
        .await?;
    super::publish_stored(&state, stored).await;
    let uris =
        maidan_mcp::resource_updates::uris_for_message(state.store.as_ref(), MessageId(message_id))
            .await;
    state.mcp.publish_resource_uris(uris).await;
    Ok(StatusCode::NO_CONTENT)
}

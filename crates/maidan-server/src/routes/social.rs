//! Social-signal handlers: votes, reactions, and pins.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use chrono::Utc;
use maidan_auth::{
    capability::{WORKSPACE_READ, WORKSPACE_WRITE},
    AuthContext,
};
use maidan_router::{resolve_message_chain, resolve_thread_context};
use maidan_types::*;

use super::{cap, ensure_workspace, publish, ApiResult};
use crate::dto::*;
use crate::error::ApiJson;
use crate::state::AppState;

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

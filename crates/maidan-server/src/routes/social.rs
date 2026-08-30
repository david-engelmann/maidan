//! Social-signal handlers: votes, reactions, and pins.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use maidan_auth::{
    capability::{WORKSPACE_READ, WORKSPACE_WRITE},
    AuthContext,
};
use maidan_router::{resolve_message_chain, resolve_thread_context};
use maidan_types::*;

use super::{cap, ensure_workspace, publish_stored, ApiResult};
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
    maidan_auth::ensure_thread_access(state.store.as_ref(), &auth, chain.thread_id).await?;
    super::ensure_acting_member(&auth, MemberId(body.member_id))?;
    if let Some(c) = body.confidence {
        if !(0.0..=1.0).contains(&c) {
            return Err(crate::error::ApiError::BadRequest(
                "confidence must be in 0..=1".into(),
            ));
        }
    }
    // Cluster 206: vote row + `VoteCast` event commit atomically (transactional
    // outbox); `publish_stored` then notifies the bus.
    let stored = state
        .store
        .cast_vote_with_event(NewVote {
            message_id: MessageId(message_id),
            member_id: MemberId(body.member_id),
            kind: body.kind.clone(),
            confidence: body.confidence,
        })
        .await?;
    publish_stored(&state, stored).await;
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
    maidan_auth::ensure_thread_access(state.store.as_ref(), &auth, chain.thread_id).await?;
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
    maidan_auth::ensure_thread_access(state.store.as_ref(), &auth, chain.thread_id).await?;
    let member_id = MemberId(body.member_id);
    super::ensure_acting_member(&auth, member_id)?;
    let emoji = body.emoji.clone();
    // Cluster 206: reaction row + `ReactionAdded` event commit atomically.
    let stored = state
        .store
        .add_reaction_with_event(NewReaction {
            message_id,
            member_id,
            emoji,
        })
        .await?;
    publish_stored(&state, stored).await;
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
    maidan_auth::ensure_thread_access(state.store.as_ref(), &auth, chain.thread_id).await?;
    let member_id = MemberId(body.member_id);
    super::ensure_acting_member(&auth, member_id)?;
    let emoji = body.emoji.clone();
    // Cluster 206: the DELETE + `ReactionRemoved` event commit atomically; the
    // event is only produced when a row was actually removed.
    let (_removed, stored) = state
        .store
        .remove_reaction_with_event(message_id, member_id, &emoji)
        .await?;
    if let Some(stored) = stored {
        publish_stored(&state, stored).await;
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
    maidan_auth::ensure_thread_access(state.store.as_ref(), &auth, chain.thread_id).await?;
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
    maidan_auth::ensure_thread_access(state.store.as_ref(), &auth, thread_id).await?;
    let message_id = MessageId(body.message_id);
    let member_id = MemberId(body.member_id);
    super::ensure_acting_member(&auth, member_id)?;
    let stored = state
        .store
        .pin_message_with_event(NewPin {
            thread_id,
            message_id,
            member_id,
        })
        .await?;
    super::publish_stored(&state, stored).await;
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
    maidan_auth::ensure_thread_access(state.store.as_ref(), &auth, thread_id).await?;
    let message_id = MessageId(body.message_id);
    let member_id = MemberId(body.member_id);
    super::ensure_acting_member(&auth, member_id)?;
    let (removed, stored) = state
        .store
        .unpin_message_with_event(thread_id, message_id, member_id)
        .await?;
    if removed {
        if let Some(stored) = stored {
            super::publish_stored(&state, stored).await;
        }
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
    maidan_auth::ensure_thread_access(state.store.as_ref(), &auth, ThreadId(thread_id)).await?;
    Ok(Json(
        state
            .store
            .list_pins_for_thread(ThreadId(thread_id))
            .await?,
    ))
}

//! Direct message HTTP routes and subscribe filter expansion.

use crate::routes::publish_routed_mentions;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Extension, Json,
};
use maidan_auth::capability::{MESSAGE_POST, WORKSPACE_READ};
use maidan_auth::AuthContext;
use maidan_types::*;
use serde::Deserialize;

use crate::error::ApiError;
use crate::state::AppState;

type ApiResult<T> = Result<T, ApiError>;

fn cap(auth: &AuthContext, capability: &str) -> Result<(), ApiError> {
    auth.require_capability(capability).map_err(Into::into)
}

fn ensure_workspace(auth: &AuthContext, workspace_id: WorkspaceId) -> Result<(), ApiError> {
    auth.ensure_workspace(workspace_id).map_err(Into::into)
}

#[derive(Debug, Deserialize)]
pub struct DmMemberQuery {
    pub member_id: uuid::Uuid,
}

#[derive(Debug, Deserialize)]
pub struct OpenDmBody {
    pub member_id: uuid::Uuid,
    pub other_member_id: uuid::Uuid,
}

fn ensure_dm_participant(dm: &DmConversation, member_id: MemberId) -> Result<(), ApiError> {
    if dm.member_low_id == member_id || dm.member_high_id == member_id {
        Ok(())
    } else {
        Err(ApiError::Forbidden(
            "member is not a participant in this DM".into(),
        ))
    }
}

pub async fn dm_conversation_id_for_thread(
    store: &dyn maidan_store::Store,
    thread_id: ThreadId,
) -> Option<DmConversationId> {
    store
        .dm_conversation_for_thread(thread_id)
        .await
        .ok()
        .flatten()
        .map(|d| d.id)
}

pub async fn expand_event_filter(
    state: &AppState,
    filter: &mut EventFilter,
    auth: &AuthContext,
) -> Result<(), ApiError> {
    if let Some(dm_id) = filter.dm_conversation_id {
        let dm = state
            .store
            .get_dm_conversation(dm_id)
            .await
            .map_err(ApiError::from)?;
        filter.workspace_id = Some(filter.workspace_id.unwrap_or(dm.workspace_id));
        filter.thread_id = Some(dm.thread_id);
    }
    // Cluster 203: a subscriber must be able to access the thread it filters to.
    // `ensure_thread_access` is DM-participant-aware (Cluster 180), so a
    // non-participant can no longer tail a DM / group-DM by supplying its
    // `dm_conversation_id` (expanded to `thread_id` above) or its `thread_id`
    // directly — the events analog of the 180 read gap. Bypass is exempt.
    if let Some(thread_id) = filter.thread_id {
        if !auth.bypass {
            maidan_auth::ensure_thread_access(state.store.as_ref(), auth, thread_id)
                .await
                .map_err(ApiError::from)?;
        }
    }
    Ok(())
}

pub async fn open_dm_conversation(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(workspace_id): Path<uuid::Uuid>,
    Json(body): Json<OpenDmBody>,
) -> ApiResult<Json<DmConversation>> {
    cap(&auth, WORKSPACE_READ)?;
    let workspace_id = WorkspaceId(workspace_id);
    ensure_workspace(&auth, workspace_id)?;
    let member_id = MemberId(body.member_id);
    let other = MemberId(body.other_member_id);
    state.store.get_member(member_id).await?;
    state.store.get_member(other).await?;
    let dm = state
        .store
        .open_dm_conversation(workspace_id, member_id, other)
        .await?;
    Ok(Json(dm))
}

pub async fn list_dm_conversations(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(workspace_id): Path<uuid::Uuid>,
    Query(q): Query<DmMemberQuery>,
) -> ApiResult<Json<Vec<DmConversation>>> {
    cap(&auth, WORKSPACE_READ)?;
    let workspace_id = WorkspaceId(workspace_id);
    ensure_workspace(&auth, workspace_id)?;
    let member_id = MemberId(q.member_id);
    // Cluster 203: a **session** caller may only enumerate its OWN DM graph
    // (otherwise a `/ui` user could list who any member messages). A bearer is
    // the orchestrator model and may list on behalf of any member (unchanged);
    // bypass is unrestricted — same rule as member-attributed writes (202).
    crate::routes::ensure_acting_member(&auth, member_id)?;
    Ok(Json(
        state
            .store
            .list_dm_conversations_for_member(workspace_id, member_id)
            .await?,
    ))
}

pub async fn get_dm_conversation(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<DmConversation>> {
    cap(&auth, WORKSPACE_READ)?;
    let dm = state
        .store
        .get_dm_conversation(DmConversationId(id))
        .await?;
    ensure_workspace(&auth, dm.workspace_id)?;
    // Cluster 203: a **session** caller must be a participant to read a DM's
    // metadata (roster + thread). Bearer = orchestrator (act-as-any); bypass
    // exempt — consistent with the write anti-spoofing guard (202).
    if !auth.bypass && auth.token_id.is_none() {
        ensure_dm_participant(&dm, auth.member_id)?;
    }
    Ok(Json(dm))
}

pub async fn post_dm_message(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
    Json(body): Json<PostDmMessage>,
) -> ApiResult<(StatusCode, Json<Message>)> {
    cap(&auth, MESSAGE_POST)?;
    let dm = state
        .store
        .get_dm_conversation(DmConversationId(id))
        .await?;
    ensure_workspace(&auth, dm.workspace_id)?;
    let author_id = MemberId(body.author_id);
    crate::routes::ensure_acting_member(&auth, author_id)?;
    ensure_dm_participant(&dm, author_id)?;
    let metadata = body.metadata.unwrap_or_else(|| serde_json::json!({}));
    let (m, stored) = state
        .store
        .post_message_with_event(
            NewMessage {
                thread_id: dm.thread_id,
                author_id,
                body: body.body,
                metadata,
                content: None,
            },
            Some(dm.id),
        )
        .await?;
    crate::routes::publish_stored(&state, stored).await;
    publish_routed_mentions(&state, dm.thread_id, dm.workspace_id, &m).await;
    let uris = maidan_mcp::resource_updates::uris_for_message(state.store.as_ref(), m.id).await;
    state.mcp.publish_resource_uris(uris).await;
    Ok((StatusCode::CREATED, Json(m)))
}

pub async fn list_dm_messages(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
    Query(q): Query<crate::dto::ListMessagesQuery>,
) -> ApiResult<Json<Vec<Message>>> {
    cap(&auth, WORKSPACE_READ)?;
    let dm = state
        .store
        .get_dm_conversation(DmConversationId(id))
        .await?;
    ensure_workspace(&auth, dm.workspace_id)?;
    Ok(Json(
        state.store.list_messages(dm.thread_id, q.limit).await?,
    ))
}

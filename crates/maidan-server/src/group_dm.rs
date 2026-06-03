//! Multi-member group direct message routes.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Extension, Json,
};
use chrono::Utc;
use maidan_auth::capability::{MESSAGE_POST, WORKSPACE_READ};
use maidan_auth::AuthContext;
use maidan_router::resolve_thread_context;
use maidan_types::*;

use crate::error::ApiError;
use crate::routes::{publish, publish_routed_mentions};
use crate::state::AppState;

type ApiResult<T> = Result<T, ApiError>;

fn cap(auth: &AuthContext, capability: &str) -> Result<(), ApiError> {
    auth.require_capability(capability).map_err(Into::into)
}

fn ensure_workspace(auth: &AuthContext, workspace_id: WorkspaceId) -> Result<(), ApiError> {
    auth.ensure_workspace(workspace_id).map_err(Into::into)
}

#[derive(Debug, serde::Deserialize)]
pub struct GroupDmMemberQuery {
    pub member_id: uuid::Uuid,
}

pub async fn open_group_dm(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(workspace_id): Path<uuid::Uuid>,
    Json(body): Json<OpenGroupDmBody>,
) -> ApiResult<(StatusCode, Json<GroupDmConversation>)> {
    cap(&auth, WORKSPACE_READ)?;
    let workspace_id = WorkspaceId(workspace_id);
    ensure_workspace(&auth, workspace_id)?;
    let member_ids: Vec<MemberId> = body.member_ids.into_iter().map(MemberId).collect();
    let group = state
        .store
        .open_group_dm_conversation(workspace_id, &member_ids, body.title)
        .await?;
    Ok((StatusCode::CREATED, Json(group)))
}

pub async fn list_group_dms(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(workspace_id): Path<uuid::Uuid>,
    Query(q): Query<GroupDmMemberQuery>,
) -> ApiResult<Json<Vec<GroupDmConversation>>> {
    cap(&auth, WORKSPACE_READ)?;
    let workspace_id = WorkspaceId(workspace_id);
    ensure_workspace(&auth, workspace_id)?;
    Ok(Json(
        state
            .store
            .list_group_dm_conversations_for_member(workspace_id, MemberId(q.member_id))
            .await?,
    ))
}

pub async fn get_group_dm(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
) -> ApiResult<Json<GroupDmConversation>> {
    cap(&auth, WORKSPACE_READ)?;
    let group = state
        .store
        .get_group_dm_conversation(GroupDmConversationId(id))
        .await?;
    ensure_workspace(&auth, group.workspace_id)?;
    Ok(Json(group))
}

pub async fn post_group_dm_message(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(id): Path<uuid::Uuid>,
    Json(body): Json<PostDmMessage>,
) -> ApiResult<(StatusCode, Json<Message>)> {
    cap(&auth, MESSAGE_POST)?;
    let group_id = GroupDmConversationId(id);
    let group = state.store.get_group_dm_conversation(group_id).await?;
    ensure_workspace(&auth, group.workspace_id)?;
    let author_id = MemberId(body.author_id);
    if !state.store.group_dm_has_member(group_id, author_id).await? {
        return Err(ApiError::Forbidden(
            "member is not a participant in this group DM".into(),
        ));
    }
    let metadata = body.metadata.unwrap_or_else(|| serde_json::json!({}));
    let m = state
        .store
        .post_message(NewMessage {
            thread_id: group.thread_id,
            author_id,
            body: body.body,
            metadata,
        })
        .await?;
    let ctx = resolve_thread_context(state.store.as_ref(), group.thread_id).await?;
    publish(
        &state,
        Event::MessagePosted {
            occurred_at: Utc::now(),
            workspace_id: group.workspace_id,
            channel_id: ctx.channel_id,
            thread_id: group.thread_id,
            dm_conversation_id: None,
            message: m.clone(),
        },
    )
    .await;
    publish_routed_mentions(&state, group.thread_id, group.workspace_id, &m).await;
    Ok((StatusCode::CREATED, Json(m)))
}

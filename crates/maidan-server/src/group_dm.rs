//! Multi-member group direct message routes.

use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Extension, Json,
};
use maidan_auth::capability::{MESSAGE_POST, WORKSPACE_READ};
use maidan_auth::AuthContext;
use maidan_types::*;

use crate::error::ApiError;
use crate::routes::publish_routed_mentions;
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
    let member_id = MemberId(q.member_id);
    // Cluster 203: a session caller may only list its OWN group DMs; bearer =
    // orchestrator (act-as-any); bypass unrestricted (same rule as writes, 202).
    crate::routes::ensure_acting_member(&auth, member_id)?;
    Ok(Json(
        state
            .store
            .list_group_dm_conversations_for_member(workspace_id, member_id)
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
    // Cluster 203: a session caller must be a participant to read a group DM's
    // metadata; bearer = orchestrator (act-as-any); bypass exempt.
    if !auth.bypass && auth.token_id.is_none() && !group.member_ids.contains(&auth.member_id) {
        return Err(ApiError::Forbidden(
            "member is not a participant in this group DM".into(),
        ));
    }
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
    crate::routes::ensure_acting_member(&auth, author_id)?;
    if !state.store.group_dm_has_member(group_id, author_id).await? {
        return Err(ApiError::Forbidden(
            "member is not a participant in this group DM".into(),
        ));
    }
    let metadata = body.metadata.unwrap_or_else(|| serde_json::json!({}));
    let (m, stored) = state
        .store
        .post_message_with_event(
            NewMessage {
                thread_id: group.thread_id,
                author_id,
                body: body.body,
                metadata,
                content: None,
            },
            None,
        )
        .await?;
    crate::routes::publish_stored(&state, stored).await;
    publish_routed_mentions(&state, group.thread_id, group.workspace_id, &m).await;
    Ok((StatusCode::CREATED, Json(m)))
}

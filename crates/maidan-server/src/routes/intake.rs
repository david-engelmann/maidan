//! A workspace's intake queue: incoming work proposals from agents, humans
//! and integrations that are not tasks yet. Proposing announces
//! `IntakeProposed`. Triage is audited: accepting links the item to a thread
//! (an existing one, or a new one created from the item) and announces
//! `IntakeAccepted`; rejecting records the reason and announces
//! `IntakeRejected`. An item is triaged once.

use axum::{extract::State, http::StatusCode, Extension, Json};
use maidan_auth::{
    capability::{WORKSPACE_READ, WORKSPACE_WRITE},
    AuthContext,
};
use maidan_router::resolve_channel_context;
use maidan_types::*;

use super::{cap, ensure_workspace, observe_spawn_denial, publish_stored, ApiResult};
use crate::dto::*;
use crate::error::ApiError;
use crate::extract::{ApiJson, ApiPath, ApiQuery};
use crate::state::AppState;

const DEFAULT_LIMIT: i64 = 50;
const MAX_LIMIT: i64 = 200;

/// `POST /workspaces/:wid/intake` — propose an item. `201` with the item.
pub async fn create_intake_item(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(wid): ApiPath<uuid::Uuid>,
    ApiJson(body): ApiJson<CreateIntakeItem>,
) -> ApiResult<(StatusCode, Json<IntakeItem>)> {
    let workspace_id = WorkspaceId(wid);
    cap(&auth, WORKSPACE_WRITE)?;
    ensure_workspace(&auth, workspace_id)?;
    state.store.get_workspace(workspace_id).await?;
    let title = body.title.trim();
    if title.is_empty() {
        return Err(ApiError::BadRequest("title must not be blank".into()));
    }
    let source = match body.source {
        Some(source) => source,
        None if auth.app_installation_id.is_some() => IntakeSource::Integration,
        None => match state.store.get_member(auth.member_id).await {
            Ok(member) if member.kind == MemberKind::Agent => IntakeSource::Agent,
            _ => IntakeSource::Human,
        },
    };
    let (item, stored) = state
        .store
        .create_intake_item(NewIntakeItem {
            workspace_id,
            title: title.to_string(),
            body: body.body.filter(|b| !b.trim().is_empty()),
            source,
            proposed_by: auth.member_id,
        })
        .await?;
    publish_stored(&state, stored).await;
    Ok((StatusCode::CREATED, Json(item)))
}

/// `GET /workspaces/:wid/intake` — items newest first, optionally in one
/// state, paged by `cursor`.
pub async fn list_intake_items(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(wid): ApiPath<uuid::Uuid>,
    ApiQuery(q): ApiQuery<ListIntakeQuery>,
) -> ApiResult<Json<Vec<IntakeItem>>> {
    let workspace_id = WorkspaceId(wid);
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, workspace_id)?;
    state.store.get_workspace(workspace_id).await?;
    let limit = q.limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT);
    Ok(Json(
        state
            .store
            .list_intake_items(workspace_id, q.state, q.cursor, limit)
            .await?,
    ))
}

/// `GET /workspaces/:wid/intake/:id` — one item, `404` if not in the workspace.
pub async fn get_intake_item(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath((wid, id)): ApiPath<(uuid::Uuid, uuid::Uuid)>,
) -> ApiResult<Json<IntakeItem>> {
    let workspace_id = WorkspaceId(wid);
    cap(&auth, WORKSPACE_READ)?;
    ensure_workspace(&auth, workspace_id)?;
    match state.store.get_intake_item(workspace_id, id).await? {
        Some(item) => Ok(Json(item)),
        None => Err(ApiError::NotFound),
    }
}

/// `POST /workspaces/:wid/intake/:id/triage` — accept or reject a proposed
/// item. `409` when it was already triaged.
pub async fn triage_intake_item(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath((wid, id)): ApiPath<(uuid::Uuid, uuid::Uuid)>,
    ApiJson(body): ApiJson<TriageIntakeItem>,
) -> ApiResult<Json<IntakeItem>> {
    let workspace_id = WorkspaceId(wid);
    cap(&auth, WORKSPACE_WRITE)?;
    ensure_workspace(&auth, workspace_id)?;
    let Some(item) = state.store.get_intake_item(workspace_id, id).await? else {
        return Err(ApiError::NotFound);
    };
    if item.state != IntakeState::Proposed {
        return Err(ApiError::Conflict("intake item already triaged".into()));
    }
    let decision = match body.decision {
        IntakeTriageDecision::Reject => {
            let reason = body.reason.as_deref().map(str::trim).unwrap_or_default();
            if reason.is_empty() {
                return Err(ApiError::BadRequest(
                    "rejecting an intake item needs a reason".into(),
                ));
            }
            IntakeDecision::Reject {
                reason: reason.to_string(),
            }
        }
        IntakeTriageDecision::Accept => IntakeDecision::Accept {
            thread_id: accepted_thread(&state, &auth, workspace_id, &item, &body).await?,
        },
    };
    let (action, metadata) = match &decision {
        IntakeDecision::Accept { thread_id } => (
            "intake.accept",
            serde_json::json!({ "thread_id": thread_id.0 }),
        ),
        IntakeDecision::Reject { reason } => {
            ("intake.reject", serde_json::json!({ "reason": reason }))
        }
    };
    let (item, stored) = state
        .store
        .triage_intake_item_audited(
            workspace_id,
            id,
            auth.member_id,
            decision,
            NewAuditEvent {
                scope: AuditScope::Workspace(workspace_id),
                actor_id: Some(auth.actor_id),
                action: action.into(),
                target_kind: Some("intake_item".into()),
                target_id: Some(id),
                metadata,
            },
        )
        .await?;
    publish_stored(&state, stored).await;
    Ok(Json(item))
}

/// The thread an accepted item links to: the named existing thread, which
/// must be in the workspace, or a new thread in the named channel.
async fn accepted_thread(
    state: &AppState,
    auth: &AuthContext,
    workspace_id: WorkspaceId,
    item: &IntakeItem,
    body: &TriageIntakeItem,
) -> ApiResult<ThreadId> {
    match (body.thread_id, body.channel_id) {
        (Some(thread_id), None) => {
            let scope =
                maidan_auth::authorize_thread(state.store.as_ref(), auth, ThreadId(thread_id))
                    .await?;
            if scope.workspace_id != workspace_id {
                return Err(ApiError::NotFound);
            }
            Ok(scope.thread_id)
        }
        (None, Some(channel_id)) => {
            let ctx = resolve_channel_context(state.store.as_ref(), ChannelId(channel_id)).await?;
            if ctx.workspace_id != workspace_id {
                return Err(ApiError::NotFound);
            }
            maidan_auth::ensure_channel_access(state.store.as_ref(), auth, ctx.channel_id).await?;
            let created = state
                .store
                .create_thread_with_footprint(
                    NewThread {
                        channel_id: ctx.channel_id,
                        parent_thread_id: None,
                        title: Some(item.title.clone()),
                        description: item.body.clone(),
                        created_by: auth.attribution().map(|a| a.subject_id),
                    },
                    None,
                    None,
                    None,
                )
                .await;
            let actor = (!auth.bypass).then_some(auth.member_id);
            let (thread, stored) = observe_spawn_denial(state, actor, created).await?;
            publish_stored(state, stored).await;
            Ok(thread.id)
        }
        _ => Err(ApiError::BadRequest(
            "accepting needs exactly one of thread_id or channel_id".into(),
        )),
    }
}

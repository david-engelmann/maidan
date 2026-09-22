//! Operator lifecycle for time-boxed cross-organization share tickets. The
//! consumer surface is deliberately separate and unauthenticated by ordinary
//! workspace bearer middleware; this module only issues, lists, and revokes.

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use maidan_auth::{capability::TOKEN_ADMIN, hash_secret, AuthContext, ShareTicketSecret};
use maidan_types::{
    ChannelId, MemberId, NewAuditEvent, NewShareTicket, ShareTicketId, WorkspaceId,
};

use super::{cap, ensure_workspace, ApiResult};
use crate::dto::{CreateShareTicket, MintShareTicketResponse, ShareTicketResponse};
use crate::error::{ApiError, ApiJson};
use crate::state::AppState;

pub async fn create_share_ticket(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(wid): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<CreateShareTicket>,
) -> ApiResult<(StatusCode, Json<MintShareTicketResponse>)> {
    let workspace_id = WorkspaceId(wid);
    cap(&auth, TOKEN_ADMIN)?;
    ensure_workspace(&auth, workspace_id)?;
    let secret = ShareTicketSecret::generate();
    let ticket = state
        .store
        .create_share_ticket(NewShareTicket {
            workspace_id,
            channel_id: ChannelId(body.channel_id),
            owner_id: MemberId(body.owner_id),
            created_by: auth.member_id,
            token_hash: hash_secret(secret.as_str()),
            expires_at: body.expires_at,
            artifact_shas: body.artifact_shas.clone(),
        })
        .await?;
    let artifact_shas = state.store.list_share_ticket_artifacts(ticket.id).await?;
    crate::audit::record(
        &state,
        NewAuditEvent {
            actor_id: Some(auth.member_id),
            action: "share_ticket.create".into(),
            target_kind: Some("share_ticket".into()),
            target_id: Some(ticket.id.0),
            metadata: serde_json::json!({
                "workspace_id": workspace_id.0,
                "channel_id": ticket.channel_id.0,
                "owner_id": ticket.owner_id.0,
                "expires_at": ticket.expires_at,
                "artifact_count": artifact_shas.len(),
            }),
        },
    )
    .await;
    Ok((
        StatusCode::CREATED,
        Json(MintShareTicketResponse {
            ticket,
            artifact_shas,
            secret: secret.as_str().to_owned(),
        }),
    ))
}

pub async fn list_share_tickets(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(wid): Path<uuid::Uuid>,
) -> ApiResult<Json<Vec<ShareTicketResponse>>> {
    let workspace_id = WorkspaceId(wid);
    cap(&auth, TOKEN_ADMIN)?;
    ensure_workspace(&auth, workspace_id)?;
    let tickets = state.store.list_share_tickets(workspace_id).await?;
    let mut response = Vec::with_capacity(tickets.len());
    for ticket in tickets {
        let artifact_shas = state.store.list_share_ticket_artifacts(ticket.id).await?;
        response.push(ShareTicketResponse {
            ticket,
            artifact_shas,
        });
    }
    Ok(Json(response))
}

pub async fn revoke_share_ticket(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path((wid, tid)): Path<(uuid::Uuid, uuid::Uuid)>,
) -> ApiResult<StatusCode> {
    let workspace_id = WorkspaceId(wid);
    let ticket_id = ShareTicketId(tid);
    cap(&auth, TOKEN_ADMIN)?;
    ensure_workspace(&auth, workspace_id)?;
    if !state
        .store
        .revoke_share_ticket(workspace_id, ticket_id)
        .await?
    {
        return Err(ApiError::NotFound);
    }
    crate::audit::record(
        &state,
        NewAuditEvent {
            actor_id: Some(auth.member_id),
            action: "share_ticket.revoke".into(),
            target_kind: Some("share_ticket".into()),
            target_id: Some(ticket_id.0),
            metadata: serde_json::json!({ "workspace_id": workspace_id.0 }),
        },
    )
    .await;
    Ok(StatusCode::NO_CONTENT)
}

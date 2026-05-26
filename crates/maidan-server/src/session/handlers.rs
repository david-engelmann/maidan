use axum::{extract::State, Extension, Json};
use chrono::{DateTime, Utc};
use maidan_types::{MemberId, WorkspaceId};
use serde::Serialize;
use utoipa::ToSchema;

use crate::error::ApiError;
use crate::session::SessionContext;
use crate::state::AppState;

#[derive(Debug, Serialize, ToSchema)]
pub struct SessionResponse {
    pub member_id: MemberId,
    pub workspace_id: WorkspaceId,
    pub expires_at: DateTime<Utc>,
}

pub async fn get_session(
    State(state): State<AppState>,
    Extension(ctx): Extension<SessionContext>,
) -> Result<Json<SessionResponse>, ApiError> {
    let session = state.store.get_session(ctx.session_id).await?;
    if session.expires_at < Utc::now() {
        let _ = state.store.delete_session(session.id).await;
        return Err(ApiError::Unauthorized);
    }
    Ok(Json(SessionResponse {
        member_id: session.member_id,
        workspace_id: session.workspace_id,
        expires_at: session.expires_at,
    }))
}

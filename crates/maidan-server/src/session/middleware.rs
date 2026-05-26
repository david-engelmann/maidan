//! Session cookie validation middleware.

use axum::{
    extract::{Request, State},
    middleware::Next,
    response::{IntoResponse, Response},
};
use chrono::Utc;

use crate::error::ApiError;
use crate::session::{parse_session_cookie, SessionContext};
use crate::state::AppState;

pub async fn load_session(
    state: &AppState,
    headers: &axum::http::HeaderMap,
) -> Result<SessionContext, ApiError> {
    let session_id = parse_session_cookie(headers).ok_or(ApiError::Unauthorized)?;
    let session = state
        .store
        .get_session(session_id)
        .await
        .map_err(|_| ApiError::Unauthorized)?;
    if session.expires_at < Utc::now() {
        let _ = state.store.delete_session(session.id).await;
        return Err(ApiError::Unauthorized);
    }
    Ok(SessionContext {
        session_id: session.id,
        member_id: session.member_id,
        workspace_id: session.workspace_id,
    })
}

pub async fn require_middleware(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Response {
    match load_session(&state, req.headers()).await {
        Ok(ctx) => {
            req.extensions_mut().insert(ctx);
            next.run(req).await
        }
        Err(err) => err.into_response(),
    }
}

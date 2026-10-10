//! Session cookie validation middleware.

use axum::{
    extract::{Request, State},
    middleware::Next,
    response::{IntoResponse, Response},
};
use chrono::Utc;
use maidan_auth::{resolve_token_id, AuthError};
use maidan_store::StoreError;
use maidan_types::{AuditScope, NewAuditEvent};

use crate::error::ApiError;
use crate::session::{check_request_origin, parse_session_cookie, SessionContext};
use crate::state::AppState;

/// The session the request's cookie names, with the authority of the token it
/// was made from resolved again now. A session whose token is no longer live
/// (revoked, rotated, expired, its grant or app installation gone) is deleted
/// and refused, so it ends when the token does rather than at its own expiry.
/// So is a session whose member the identity provider has deactivated through
/// SCIM: deactivation revokes the member's tokens, and a session the person
/// signed in to has no token to lose, so it is ended here instead.
pub async fn load_session(
    state: &AppState,
    headers: &axum::http::HeaderMap,
) -> Result<SessionContext, ApiError> {
    let settings = state.browser_sessions().ok_or(ApiError::Unauthorized)?;
    let session_id =
        parse_session_cookie(headers, &settings.secret).ok_or(ApiError::Unauthorized)?;
    let session = state
        .store
        .get_session(session_id)
        .await
        .map_err(|_| ApiError::Unauthorized)?;
    if session.expires_at < Utc::now() {
        let _ = state.store.delete_expired_session(session.id).await;
        return Err(ApiError::Unauthorized);
    }
    match state.store.get_scim_user(session.member_id).await {
        Ok(Some(user)) if !user.active => {
            let _ = state
                .store
                .delete_session_audited(
                    session.id,
                    Box::new(|ended| NewAuditEvent {
                        scope: AuditScope::Workspace(ended.workspace_id),
                        actor_id: Some(ended.member_id),
                        action: "session.delete".into(),
                        target_kind: Some("member".into()),
                        target_id: Some(ended.member_id.0),
                        metadata: serde_json::json!({
                            "workspace_id": ended.workspace_id.0,
                            "reason": "member_deactivated",
                        }),
                    }),
                )
                .await;
            return Err(ApiError::Unauthorized);
        }
        Ok(_) => {}
        // A failed lookup cannot show the member is active.
        Err(_) => return Err(ApiError::Unauthorized),
    }
    let token = match session.api_token_id {
        None => None,
        Some(token_id) => match resolve_token_id(state.store.as_ref(), token_id).await {
            Ok(ctx) if ctx.workspace_id == session.workspace_id => Some(ctx),
            Ok(_) | Err(AuthError::Unauthorized) | Err(AuthError::Store(StoreError::NotFound)) => {
                // The token is gone, so the session already grants nothing.
                // Record the end in the delete's transaction; a failed write
                // still refuses this request and the next one tries again.
                let _ = state
                    .store
                    .delete_session_audited(
                        session.id,
                        Box::new(|ended| NewAuditEvent {
                            scope: AuditScope::Workspace(ended.workspace_id),
                            actor_id: Some(ended.member_id),
                            action: "session.delete".into(),
                            target_kind: Some("member".into()),
                            target_id: Some(ended.member_id.0),
                            metadata: serde_json::json!({
                                "workspace_id": ended.workspace_id.0,
                                "reason": "token_ended",
                            }),
                        }),
                    )
                    .await;
                return Err(ApiError::Unauthorized);
            }
            // A failed lookup says nothing about the token: refuse this
            // request, but keep the session.
            Err(_) => return Err(ApiError::Unauthorized),
        },
    };
    Ok(SessionContext {
        session_id: session.id,
        member_id: session.member_id,
        workspace_id: session.workspace_id,
        token,
    })
}

pub async fn require_middleware(
    State(state): State<AppState>,
    mut req: Request,
    next: Next,
) -> Response {
    let session = match load_session(&state, req.headers()).await {
        Ok(ctx) => ctx,
        Err(err) => return err.into_response(),
    };
    if let Err(err) = check_request_origin(req.method(), req.headers()) {
        return err.into_response();
    }
    req.extensions_mut().insert(session);
    next.run(req).await
}

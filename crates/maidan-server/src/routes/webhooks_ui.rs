//! The console's twins of the outbound webhook routes.
//!
//! A webhook receives every matching event in the workspace, private channels
//! and DMs included, so the console holds a person to the bar the bearer
//! routes hold a token to. A bearer on these routes is held to that bar
//! exactly: `workspace:admin` to create, `workspace:read` to list and
//! `workspace:write` to revoke. A person signed in is let in when they are a
//! human member holding `workspace:admin` on a live token of their own, or the
//! workspace's first admin before anyone holds one: the session lends them
//! nothing a token of theirs could not do. An agent's sign-in never qualifies,
//! and a session made from a token is sent to the bearer routes, where its own
//! grant decides.

use axum::{extract::State, http::StatusCode, Extension, Json};
use maidan_auth::{
    capability::{WORKSPACE_ADMIN, WORKSPACE_READ, WORKSPACE_WRITE},
    AuthContext,
};
use maidan_types::{ApiTokenId, MemberId, MemberKind, WebhookSubscriptionId, WorkspaceId};

use super::{cap, ensure_workspace, ApiResult};
use crate::dto::{CreateWebhook, MintWebhookResponse, WebhookResponse};
use crate::error::ApiError;
use crate::extract::{ApiJson, ApiPath};
use crate::session::SessionContext;
use crate::state::AppState;

/// What the caller is asking to do.
#[derive(Clone, Copy)]
enum Act {
    Create,
    List,
    Revoke,
}

impl Act {
    /// The capability the bearer route for this act asks of a token.
    fn capability(self) -> &'static str {
        match self {
            Self::Create => WORKSPACE_ADMIN,
            Self::List => WORKSPACE_READ,
            Self::Revoke => WORKSPACE_WRITE,
        }
    }
}

async fn console_webhook_admin(
    state: &AppState,
    auth: &AuthContext,
    session: Option<&SessionContext>,
    workspace_id: WorkspaceId,
    act: Act,
) -> ApiResult<()> {
    let Some(session) = session else {
        cap(auth, act.capability())?;
        return ensure_workspace(auth, workspace_id);
    };
    ensure_workspace(auth, workspace_id)?;
    if session.token.is_some() {
        return Err(ApiError::Forbidden(
            "a session made from a token cannot use webhooks here; that token's holder uses /workspaces/{wid}/webhooks".into(),
        ));
    }
    let member = state
        .store
        .get_member_in(workspace_id, session.member_id)
        .await?;
    if member.kind == MemberKind::Human
        && super::member::session_holds_or_may_mint(state, session, WORKSPACE_ADMIN).await?
    {
        Ok(())
    } else {
        Err(ApiError::Forbidden(
            "only a workspace admin can see or change the workspace's webhooks".into(),
        ))
    }
}

/// The member's own token a webhook made on their session acts as: a live one
/// holding both `workspace:admin` and `workspace:read`. The one with no expiry
/// if there is one, else the latest expiry, ties going to the newest. `403`
/// when they hold none.
async fn own_webhook_token(
    state: &AppState,
    workspace_id: WorkspaceId,
    member_id: MemberId,
) -> ApiResult<ApiTokenId> {
    let now = chrono::Utc::now();
    state
        .store
        .list_api_tokens_for_member(workspace_id, member_id)
        .await?
        .into_iter()
        .filter(|t| {
            t.revoked_at.is_none()
                && t.expires_at.is_none_or(|at| at > now)
                && [WORKSPACE_ADMIN, WORKSPACE_READ]
                    .iter()
                    .all(|needed| t.capabilities.iter().any(|c| c == needed))
        })
        // `None` (no expiry) sorts last, so the maximum is the one that never
        // expires, then the latest expiry, then the newest.
        .max_by_key(|t| (t.expires_at.is_none(), t.expires_at, t.created_at))
        .map(|t| t.id)
        .ok_or_else(|| {
            ApiError::Forbidden(
                "a webhook acts as one of your tokens, and you hold none with workspace:admin and workspace:read".into(),
            )
        })
}

/// `GET /ui/api/workspaces/:wid/webhooks`.
pub async fn ui_list_webhooks(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    session: Option<Extension<SessionContext>>,
    ApiPath(wid): ApiPath<uuid::Uuid>,
) -> ApiResult<Json<Vec<WebhookResponse>>> {
    let workspace_id = WorkspaceId(wid);
    console_webhook_admin(
        &state,
        &auth,
        session.as_ref().map(|Extension(s)| s),
        workspace_id,
        Act::List,
    )
    .await?;
    crate::webhooks::list(&state, workspace_id).await
}

/// `POST /ui/api/workspaces/:wid/webhooks`.
pub async fn ui_create_webhook(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    session: Option<Extension<SessionContext>>,
    ApiPath(wid): ApiPath<uuid::Uuid>,
    ApiJson(body): ApiJson<CreateWebhook>,
) -> ApiResult<(StatusCode, Json<MintWebhookResponse>)> {
    let workspace_id = WorkspaceId(wid);
    console_webhook_admin(
        &state,
        &auth,
        session.as_ref().map(|Extension(s)| s),
        workspace_id,
        Act::Create,
    )
    .await?;
    let created_by_token = match session.as_ref() {
        // A bearer on the twin acts as its own token, as on the bearer route.
        None => auth.token_id().ok_or_else(|| {
            ApiError::Forbidden(
                "a webhook is made with a token, which this caller does not hold".into(),
            )
        })?,
        // A person signed in acts as one of their own tokens, as a webhook made
        // with that token does: it delivers while the token is live and holds
        // the capability, and ends when the token is revoked or the member is
        // deactivated. Nothing is minted.
        Some(Extension(session)) => {
            own_webhook_token(&state, workspace_id, session.member_id).await?
        }
    };
    crate::webhooks::create(&state, &auth, workspace_id, body, created_by_token).await
}

/// `DELETE /ui/api/workspaces/:wid/webhooks/:whid`.
pub async fn ui_revoke_webhook(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    session: Option<Extension<SessionContext>>,
    ApiPath((wid, whid)): ApiPath<(uuid::Uuid, uuid::Uuid)>,
) -> ApiResult<StatusCode> {
    let workspace_id = WorkspaceId(wid);
    console_webhook_admin(
        &state,
        &auth,
        session.as_ref().map(|Extension(s)| s),
        workspace_id,
        Act::Revoke,
    )
    .await?;
    crate::webhooks::revoke(&state, workspace_id, WebhookSubscriptionId(whid)).await
}

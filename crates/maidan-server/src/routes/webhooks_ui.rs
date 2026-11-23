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
use maidan_types::{MemberKind, WebhookSubscriptionId, WorkspaceId};

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
    let authority = match session.as_ref() {
        // A bearer on the twin acts as its own token, as on the bearer route.
        None => crate::webhooks::Authority::Token(auth.token_id().ok_or_else(|| {
            ApiError::Forbidden(
                "a webhook is made with a token, which this caller does not hold".into(),
            )
        })?),
        // A person signed in gets a token minted for the webhook. They must be
        // able to hold both capabilities it carries.
        Some(Extension(session)) => {
            for needed in [WORKSPACE_ADMIN, WORKSPACE_READ] {
                if !super::member::session_holds_or_may_mint(&state, session, needed).await? {
                    return Err(ApiError::Forbidden(format!(
                        "a webhook needs {needed}, which this sign-in cannot hold"
                    )));
                }
            }
            crate::webhooks::Authority::MintFor(session.member_id)
        }
    };
    crate::webhooks::create(&state, &auth, workspace_id, body, authority).await
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
    crate::webhooks::revoke(&state, &auth, workspace_id, WebhookSubscriptionId(whid)).await
}

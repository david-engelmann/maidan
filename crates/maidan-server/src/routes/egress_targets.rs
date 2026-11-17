//! The egress trust boundary: the per-workspace allowlist of external
//! destinations Maidan may deliver a result to.
//!
//! **Why this surface is `token:admin` throughout, reads included.** The
//! allowlist is policy, not status. A result's `deliver_to` is written by an
//! agent holding `thread:transition`, so letting a workspace-scoped token
//! *enumerate* the allowlist would hand an agent the list of destinations worth
//! aiming at. A producer that wants to know where its result actually landed
//! reads the per-thread delivery status instead — the disposition, not the
//! policy.
//!
//! An operator's loop is list → bless → revoke-by-id. The id is a surrogate
//! precisely so the revoke is routable: a GitHub selector is `owner/name`, and
//! a `/` in a path segment is not.
//!
//! The console's twins under `/ui/api` hold a person signed in to the same
//! line: an OIDC session carries no `token:admin`, so it is let in only when
//! that person already holds it on a live token of their own, or may mint the
//! first one, as the console's agent and invite routes decide.

use axum::{extract::State, http::StatusCode, Extension, Json};
use maidan_auth::{capability::TOKEN_ADMIN, AuthContext};
use maidan_types::{
    AllowedEgressTarget, AuditScope, EgressTargetId, MemberKind, NewAuditEvent, NewEgressTarget,
    WorkspaceId,
};

use super::{cap, ensure_workspace, ApiResult};
use crate::dto::AllowEgressTarget;
use crate::error::ApiError;
use crate::extract::{ApiJson, ApiPath};
use crate::session::SessionContext;
use crate::state::AppState;

/// `POST /workspaces/:wid/egress-targets` — bless a destination. Idempotent: a
/// re-bless returns the existing entry rather than a duplicate. `400` when the
/// selector is a name rather than an id (the store validates, so every write
/// path inherits the rule).
pub async fn allow_egress_target(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(wid): ApiPath<uuid::Uuid>,
    ApiJson(body): ApiJson<AllowEgressTarget>,
) -> ApiResult<(StatusCode, Json<AllowedEgressTarget>)> {
    let workspace_id = WorkspaceId(wid);
    bearer_admin(&auth, workspace_id)?;
    allow(&state, &auth, workspace_id, body).await
}

/// `GET /workspaces/:wid/egress-targets` — the workspace's blessed destinations.
/// An empty list is the default and means **deliver nowhere**.
pub async fn list_egress_targets(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath(wid): ApiPath<uuid::Uuid>,
) -> ApiResult<Json<Vec<AllowedEgressTarget>>> {
    let workspace_id = WorkspaceId(wid);
    bearer_admin(&auth, workspace_id)?;
    list(&state, workspace_id).await
}

/// `DELETE /workspaces/:wid/egress-targets/:tid` — revoke a blessing. `404` when
/// this workspace has no such entry; the delete is workspace-scoped, so a
/// guessed id from another workspace is a `404` rather than a cross-tenant
/// revoke.
pub async fn revoke_egress_target(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    ApiPath((wid, tid)): ApiPath<(uuid::Uuid, uuid::Uuid)>,
) -> ApiResult<StatusCode> {
    let workspace_id = WorkspaceId(wid);
    bearer_admin(&auth, workspace_id)?;
    revoke(&state, &auth, workspace_id, tid).await
}

/// `POST /ui/api/workspaces/:wid/egress-targets` — bless from the console.
pub async fn ui_allow_egress_target(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    session: Option<Extension<SessionContext>>,
    ApiPath(wid): ApiPath<uuid::Uuid>,
    ApiJson(body): ApiJson<AllowEgressTarget>,
) -> ApiResult<(StatusCode, Json<AllowedEgressTarget>)> {
    let workspace_id = WorkspaceId(wid);
    console_admin(
        &state,
        &auth,
        session.as_ref().map(|Extension(s)| s),
        workspace_id,
    )
    .await?;
    allow(&state, &auth, workspace_id, body).await
}

/// `GET /ui/api/workspaces/:wid/egress-targets` — the allowlist in the console.
pub async fn ui_list_egress_targets(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    session: Option<Extension<SessionContext>>,
    ApiPath(wid): ApiPath<uuid::Uuid>,
) -> ApiResult<Json<Vec<AllowedEgressTarget>>> {
    let workspace_id = WorkspaceId(wid);
    console_admin(
        &state,
        &auth,
        session.as_ref().map(|Extension(s)| s),
        workspace_id,
    )
    .await?;
    list(&state, workspace_id).await
}

/// `DELETE /ui/api/workspaces/:wid/egress-targets/:tid` — revoke from the console.
pub async fn ui_revoke_egress_target(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    session: Option<Extension<SessionContext>>,
    ApiPath((wid, tid)): ApiPath<(uuid::Uuid, uuid::Uuid)>,
) -> ApiResult<StatusCode> {
    let workspace_id = WorkspaceId(wid);
    console_admin(
        &state,
        &auth,
        session.as_ref().map(|Extension(s)| s),
        workspace_id,
    )
    .await?;
    revoke(&state, &auth, workspace_id, tid).await
}

fn bearer_admin(auth: &AuthContext, workspace_id: WorkspaceId) -> ApiResult<()> {
    cap(auth, TOKEN_ADMIN)?;
    ensure_workspace(auth, workspace_id)
}

/// Whether the caller of a console route may see and change the allowlist. A
/// bearer is held to `token:admin`, as on the bearer routes. A person signed
/// in qualifies when they are a human member who holds `token:admin` on a
/// live token, or who may mint the first admin token: either way they could
/// make these calls with a token, so the session lends them nothing new. An
/// agent's own sign-in never qualifies, so an agent cannot list the
/// destinations worth aiming its `deliver_to` at. A session made from a token
/// is refused, as on the console's invite routes: that token's holder uses the
/// bearer routes, where its own grant decides.
async fn console_admin(
    state: &AppState,
    auth: &AuthContext,
    session: Option<&SessionContext>,
    workspace_id: WorkspaceId,
) -> ApiResult<()> {
    let Some(session) = session else {
        return bearer_admin(auth, workspace_id);
    };
    ensure_workspace(auth, workspace_id)?;
    if session.token.is_some() {
        return Err(ApiError::Forbidden(
            "a session made from a token cannot use the allowlist here; that token's holder uses /workspaces/{wid}/egress-targets".into(),
        ));
    }
    let member = state
        .store
        .get_member_in(workspace_id, session.member_id)
        .await?;
    if member.kind == MemberKind::Human
        && super::member::session_holds_or_may_mint(state, session, TOKEN_ADMIN).await?
    {
        Ok(())
    } else {
        Err(ApiError::Forbidden(
            "only a workspace admin can see or change where results may be delivered".into(),
        ))
    }
}

async fn allow(
    state: &AppState,
    auth: &AuthContext,
    workspace_id: WorkspaceId,
    body: AllowEgressTarget,
) -> ApiResult<(StatusCode, Json<AllowedEgressTarget>)> {
    let actor = auth.actor_id();
    let target = state
        .store
        .allow_egress_target_audited(
            NewEgressTarget {
                workspace_id,
                surface: body.surface,
                selector: body.selector,
                created_by: Some(actor),
            },
            Box::new(move |target| egress_target_allowed_event(actor, target)),
        )
        .await?;
    Ok((StatusCode::CREATED, Json(target)))
}

/// The record of blessing a destination.
fn egress_target_allowed_event(
    actor: maidan_types::MemberId,
    target: &AllowedEgressTarget,
) -> NewAuditEvent {
    NewAuditEvent {
        scope: AuditScope::Workspace(target.workspace_id),
        actor_id: Some(actor),
        action: "egress_target.allow".into(),
        target_kind: Some("egress_target".into()),
        target_id: Some(target.id.0),
        metadata: serde_json::json!({
            "workspace_id": target.workspace_id.0,
            "surface": target.surface,
            "selector": target.selector,
        }),
    }
}

async fn list(
    state: &AppState,
    workspace_id: WorkspaceId,
) -> ApiResult<Json<Vec<AllowedEgressTarget>>> {
    let targets = state.store.list_egress_targets(workspace_id).await?;
    for target in targets.iter().filter(|t| t.dead.is_some()) {
        tracing::warn!(
            workspace_id = %workspace_id.0,
            id = %target.id.0,
            surface = %target.surface,
            reason = target.dead.as_deref().unwrap_or_default(),
            "egress allowlist entry is dead; revoke it"
        );
    }
    Ok(Json(targets))
}

async fn revoke(
    state: &AppState,
    auth: &AuthContext,
    workspace_id: WorkspaceId,
    tid: uuid::Uuid,
) -> ApiResult<StatusCode> {
    let revoked = state
        .store
        .revoke_egress_target_audited(
            workspace_id,
            EgressTargetId(tid),
            NewAuditEvent {
                scope: AuditScope::Workspace(workspace_id),
                actor_id: Some(auth.actor_id()),
                action: "egress_target.revoke".into(),
                target_kind: Some("egress_target".into()),
                target_id: Some(tid),
                metadata: serde_json::json!({ "workspace_id": workspace_id.0 }),
            },
        )
        .await?;
    if !revoked {
        return Err(ApiError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

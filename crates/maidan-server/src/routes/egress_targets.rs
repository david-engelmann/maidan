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

use axum::{
    extract::{Path, State},
    http::StatusCode,
    Extension, Json,
};
use maidan_auth::{capability::TOKEN_ADMIN, AuthContext};
use maidan_types::{
    AllowedEgressTarget, EgressTargetId, NewAuditEvent, NewEgressTarget, WorkspaceId,
};

use super::{cap, ensure_workspace, ApiResult};
use crate::dto::AllowEgressTarget;
use crate::error::{ApiError, ApiJson};
use crate::state::AppState;

/// `POST /workspaces/:wid/egress-targets` — bless a destination. Idempotent: a
/// re-bless returns the existing entry rather than a duplicate. `400` when the
/// selector is a name rather than an id (the store validates, so every write
/// path inherits the rule).
pub async fn allow_egress_target(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(wid): Path<uuid::Uuid>,
    ApiJson(body): ApiJson<AllowEgressTarget>,
) -> ApiResult<(StatusCode, Json<AllowedEgressTarget>)> {
    let workspace_id = WorkspaceId(wid);
    cap(&auth, TOKEN_ADMIN)?;
    ensure_workspace(&auth, workspace_id)?;
    let actor = auth.actor_id;
    let target = state
        .store
        .allow_egress_target_audited(
            NewEgressTarget {
                workspace_id,
                surface: body.surface,
                selector: body.selector,
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

/// `GET /workspaces/:wid/egress-targets` — the workspace's blessed destinations.
/// An empty list is the default and means **deliver nowhere**.
pub async fn list_egress_targets(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path(wid): Path<uuid::Uuid>,
) -> ApiResult<Json<Vec<AllowedEgressTarget>>> {
    let workspace_id = WorkspaceId(wid);
    cap(&auth, TOKEN_ADMIN)?;
    ensure_workspace(&auth, workspace_id)?;
    Ok(Json(state.store.list_egress_targets(workspace_id).await?))
}

/// `DELETE /workspaces/:wid/egress-targets/:tid` — revoke a blessing. `404` when
/// this workspace has no such entry; the delete is workspace-scoped, so a
/// guessed id from another workspace is a `404` rather than a cross-tenant
/// revoke.
pub async fn revoke_egress_target(
    State(state): State<AppState>,
    Extension(auth): Extension<AuthContext>,
    Path((wid, tid)): Path<(uuid::Uuid, uuid::Uuid)>,
) -> ApiResult<StatusCode> {
    let workspace_id = WorkspaceId(wid);
    cap(&auth, TOKEN_ADMIN)?;
    ensure_workspace(&auth, workspace_id)?;
    let revoked = state
        .store
        .revoke_egress_target_audited(
            workspace_id,
            EgressTargetId(tid),
            NewAuditEvent {
                actor_id: Some(auth.actor_id),
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

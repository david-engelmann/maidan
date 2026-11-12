//! Who a registration acts for, checked each time it fires (SEC-017).
//!
//! A webhook subscription, a slash command and an A2A push config each record
//! the API token that registered it. Each time one fires, that token is
//! resolved again as a bearer of it would be, and the registration acts only
//! while the token could register it again: not revoked, expired or rotated
//! away, its member not deactivated, in the registration's workspace, and
//! still holding the capability registering takes. Revoking or deactivating
//! the member who made a registration therefore stops it, where before it
//! fired until someone found and revoked it. A registration with no token
//! never fires. FSM hooks follow the same rule in `fsm_hooks.rs`.

use maidan_auth::AuthContext;
use maidan_types::{ApiTokenId, AuditScope, NewAuditEvent, WorkspaceId};
use serde_json::{json, Value};

use crate::state::AppState;

/// Why a registration does not act on this firing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotActing {
    /// Its creator's authority is over and will not come back: a revoked,
    /// expired or rotated token stays so, and deactivation revokes them all.
    Ended(&'static str),
    /// The check could not show the creator still holds it: a suspended
    /// workspace, or a lookup that failed. A later firing may pass.
    Held(&'static str),
}

impl NotActing {
    pub fn reason(self) -> &'static str {
        match self {
            Self::Ended(reason) | Self::Held(reason) => reason,
        }
    }
}

/// The authority a registration in `workspace_id` acts with: its creator's
/// token, resolved now, holding `capability`. `Err` says why it does not act.
pub async fn creator_authority(
    state: &AppState,
    created_by_token: Option<ApiTokenId>,
    workspace_id: WorkspaceId,
    capability: &str,
) -> Result<AuthContext, NotActing> {
    // A registration made on a browser session has no token to act as, and
    // nothing may act for nobody, so it never fires.
    let Some(token_id) = created_by_token else {
        return Err(NotActing::Ended("registered without a token"));
    };
    let auth = match maidan_auth::resolve_token_id(state.store.as_ref(), token_id).await {
        Ok(auth) => auth,
        Err(maidan_auth::AuthError::Forbidden(_)) => {
            return Err(NotActing::Held("workspace suspended"))
        }
        // A revoked, expired or rotated token, or a deactivated member's, is
        // no longer found among the live ones.
        Err(
            maidan_auth::AuthError::Unauthorized
            | maidan_auth::AuthError::Store(maidan_store::StoreError::NotFound),
        ) => return Err(NotActing::Ended("creator token no longer valid")),
        Err(maidan_auth::AuthError::Store(_)) => {
            return Err(NotActing::Held("creator check failed"))
        }
    };
    if auth.workspace_id() != workspace_id {
        return Err(NotActing::Ended("creator token is for another workspace"));
    }
    // Deactivation revokes the member's tokens in the same transaction, so
    // this only matters for a member deactivated some other way; a delegate
    // presenting the token counts as much as its subject.
    let mut members = vec![auth.member_id()];
    if auth.actor_id() != auth.member_id() {
        members.push(auth.actor_id());
    }
    for member in members {
        match state.store.get_scim_user(member).await {
            Ok(Some(user)) if !user.active => return Err(NotActing::Ended("creator deactivated")),
            Ok(_) => {}
            Err(_) => return Err(NotActing::Held("creator check failed")),
        }
    }
    if auth.require_capability(capability).is_err() {
        return Err(NotActing::Ended(
            "creator no longer holds the capability that registered it",
        ));
    }
    Ok(auth)
}

/// Record who registered something, so an administrator can tell whose a
/// registration is: the member, and the token it now acts as.
pub async fn record_registration(
    state: &AppState,
    auth: &AuthContext,
    workspace_id: WorkspaceId,
    kind: &str,
    target_id: uuid::Uuid,
) {
    crate::audit::record(
        state,
        registration_event(auth, workspace_id, kind, target_id),
    )
    .await;
}

/// The audit row [`record_registration`] writes, for a caller holding only a
/// store.
pub fn registration_event(
    auth: &AuthContext,
    workspace_id: WorkspaceId,
    kind: &str,
    target_id: uuid::Uuid,
) -> NewAuditEvent {
    NewAuditEvent {
        scope: AuditScope::Workspace(workspace_id),
        actor_id: Some(auth.actor_id()),
        action: format!("{kind}.create"),
        target_kind: Some(kind.into()),
        target_id: Some(target_id),
        metadata: json!({
            "workspace_id": workspace_id.0,
            "created_by_token": auth.token_id().map(|token| token.0),
            "member_id": Some(auth.member_id().0),
        }),
    }
}

/// Record that a registration did not act, with the reason, in its
/// workspace's audit trail, so an administrator sees which stopped and why.
pub async fn record_skip(
    state: &AppState,
    workspace_id: WorkspaceId,
    kind: &str,
    target_id: Option<uuid::Uuid>,
    reason: &str,
    mut metadata: Value,
) {
    tracing::warn!(kind, target = ?target_id, reason, "registration skipped");
    if let Some(fields) = metadata.as_object_mut() {
        fields.insert("reason".into(), json!(reason));
        fields.insert("workspace_id".into(), json!(workspace_id.0));
    }
    crate::audit::record(
        state,
        NewAuditEvent {
            scope: AuditScope::Workspace(workspace_id),
            actor_id: None,
            action: format!("{kind}.skipped"),
            target_kind: Some(kind.into()),
            target_id,
            metadata,
        },
    )
    .await;
}

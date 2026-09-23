//! Content-free authorization decision observability shared by every surface.

use std::sync::atomic::{AtomicU64, Ordering};

use maidan_store::Store;
use maidan_types::{MemberId, NewAuditEvent, WorkspaceId};
use metrics::counter;
use uuid::Uuid;

use crate::{capability, AuthContext};

const DENIAL_LOG_SAMPLE_RATE: u64 = 64;
static DENIAL_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorizationSurface {
    Rest,
    Mcp,
}

impl AuthorizationSurface {
    const fn label(self) -> &'static str {
        match self {
            Self::Rest => "rest",
            Self::Mcp => "mcp",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorizationOutcome {
    Allowed,
    Denied,
}

impl AuthorizationOutcome {
    const fn label(self) -> &'static str {
        match self {
            Self::Allowed => "allowed",
            Self::Denied => "denied",
        }
    }
}

/// A deliberately narrow authorization record. It cannot carry request bodies,
/// prompts, messages, tool arguments, provider payloads, or response content.
///
/// `subject` and `grant_id` are reserved for delegated decisions in Cluster
/// 411. Keeping them on this record ensures delegation extends this lane rather
/// than creating a second audit path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthorizationDecision {
    surface: AuthorizationSurface,
    principal: Option<MemberId>,
    subject: Option<MemberId>,
    grant_id: Option<Uuid>,
    action: &'static str,
    outcome: AuthorizationOutcome,
    resource_workspace: Option<WorkspaceId>,
}

impl AuthorizationDecision {
    pub fn capability(
        surface: AuthorizationSurface,
        auth: &AuthContext,
        capability: &'static str,
        outcome: AuthorizationOutcome,
    ) -> Self {
        Self {
            surface,
            principal: (!auth.bypass).then_some(auth.actor_id),
            subject: auth.delegation_grant_id.map(|_| auth.member_id),
            grant_id: auth.delegation_grant_id.map(|id| id.0),
            action: bounded_action(capability),
            outcome,
            resource_workspace: (!auth.bypass).then_some(auth.workspace_id),
        }
    }

    pub const fn authentication_denied(surface: AuthorizationSurface) -> Self {
        Self {
            surface,
            principal: None,
            subject: None,
            grant_id: None,
            action: "authenticate",
            outcome: AuthorizationOutcome::Denied,
            resource_workspace: None,
        }
    }

    /// Attach delegation identity without changing the record or metric shape.
    pub const fn with_delegation(mut self, subject: MemberId, grant_id: Uuid) -> Self {
        self.subject = Some(subject);
        self.grant_id = Some(grant_id);
        self
    }

    pub fn record(self) {
        counter!(
            "maidan_authorization_decisions_total",
            "surface" => self.surface.label(),
            "action" => self.action,
            "outcome" => self.outcome.label(),
            "resource" => self.resource_label(),
        )
        .increment(1);

        match self.outcome {
            AuthorizationOutcome::Allowed => self.trace_allowed(),
            AuthorizationOutcome::Denied if sample_denial() => self.trace_denied(),
            AuthorizationOutcome::Denied => {}
        }
    }

    const fn resource_label(self) -> &'static str {
        if self.resource_workspace.is_some() {
            "workspace"
        } else {
            "request"
        }
    }

    fn trace_allowed(self) {
        tracing::trace!(
            authorization.surface = self.surface.label(),
            authorization.principal = ?self.principal.map(|id| id.0),
            authorization.subject = ?self.subject.map(|id| id.0),
            authorization.grant_id = ?self.grant_id,
            authorization.action = self.action,
            authorization.outcome = self.outcome.label(),
            authorization.resource_kind = self.resource_label(),
            authorization.resource_id = ?self.resource_workspace.map(|id| id.0),
            "authorization decision"
        );
    }

    fn trace_denied(self) {
        tracing::warn!(
            authorization.surface = self.surface.label(),
            authorization.principal = ?self.principal.map(|id| id.0),
            authorization.subject = ?self.subject.map(|id| id.0),
            authorization.grant_id = ?self.grant_id,
            authorization.action = self.action,
            authorization.outcome = self.outcome.label(),
            authorization.resource_kind = self.resource_label(),
            authorization.resource_id = ?self.resource_workspace.map(|id| id.0),
            authorization.sample_rate = DENIAL_LOG_SAMPLE_RATE,
            "authorization denied (sampled)"
        );
    }
}

/// Persist one delegated authorization decision. Unlike anonymous denials,
/// delegated calls are bounded by a named, expiring, revocable grant and are
/// therefore recorded durably without sampling.
pub async fn record_delegated_authorization(
    store: &dyn Store,
    auth: &AuthContext,
    surface: AuthorizationSurface,
    action: &str,
    outcome: AuthorizationOutcome,
) {
    let Some(grant_id) = auth.delegation_grant_id else {
        return;
    };
    let event = NewAuditEvent {
        actor_id: Some(auth.actor_id),
        action: "authorization.decision".into(),
        target_kind: Some("workspace".into()),
        target_id: Some(auth.workspace_id.0),
        metadata: serde_json::json!({
            "surface": surface.label(),
            "authorization_action": action,
            "outcome": outcome.label(),
            "subject_id": auth.member_id.0,
            "grant_id": grant_id.0,
        }),
    };
    if let Err(err) = store.append_audit(event).await {
        tracing::error!(
            target: "audit",
            %err,
            actor_id = %auth.actor_id.0,
            subject_id = %auth.member_id.0,
            grant_id = %grant_id.0,
            "delegated_authorization.write_failed"
        );
    }
}

pub fn require_capability(
    auth: &AuthContext,
    surface: AuthorizationSurface,
    required: &'static str,
) -> Result<(), crate::AuthError> {
    let result = auth.require_capability(required);
    AuthorizationDecision::capability(
        surface,
        auth,
        required,
        if result.is_ok() {
            AuthorizationOutcome::Allowed
        } else {
            AuthorizationOutcome::Denied
        },
    )
    .record();
    result
}

fn sample_denial() -> bool {
    DENIAL_SEQUENCE
        .fetch_add(1, Ordering::Relaxed)
        .is_multiple_of(DENIAL_LOG_SAMPLE_RATE)
}

const fn bounded_action(action: &'static str) -> &'static str {
    if capability::is_known_const(action) {
        action
    } else {
        "unknown"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_actions_collapse_to_a_fixed_label() {
        assert_eq!(bounded_action("request-derived-value"), "unknown");
        assert_eq!(bounded_action(capability::WORKSPACE_READ), "workspace:read");
    }

    #[test]
    fn record_has_only_content_free_identity_and_decision_fields() {
        let auth = AuthContext::from_token(
            maidan_types::ApiTokenId(Uuid::new_v4()),
            MemberId(Uuid::new_v4()),
            WorkspaceId(Uuid::new_v4()),
            vec![capability::WORKSPACE_READ.to_string()],
        );
        let decision = AuthorizationDecision::capability(
            AuthorizationSurface::Rest,
            &auth,
            capability::WORKSPACE_READ,
            AuthorizationOutcome::Allowed,
        );

        assert_eq!(decision.principal, Some(auth.member_id));
        assert_eq!(decision.action, capability::WORKSPACE_READ);
        assert_eq!(decision.resource_label(), "workspace");
        assert_eq!(decision.subject, None);
        assert_eq!(decision.grant_id, None);
    }
}

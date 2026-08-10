//! Best-effort audit-trail writes for security-sensitive mutations (Cluster
//! 182).
//!
//! An audit write must never break the primary operation it records — a failed
//! audit insert must not, for example, lose a freshly minted token secret that
//! only exists in the response body. So these writes are **best-effort**: on
//! error we emit a loud `tracing::error!` (greppable `audit.write_failed`) and
//! let the operation succeed. The audit trail is a security record, not a
//! transactional participant; making it one is the dual-write concern tracked
//! for Cluster 184.
//!
//! Denied requests (401/403) are deliberately *not* written here — a rejected,
//! attacker-controlled request stream would be an unbounded audit-table write
//! amplifier. Denials are surfaced through structured logs + metrics instead.

use maidan_types::NewAuditEvent;

use crate::state::AppState;

/// Record a security-sensitive mutation to the audit trail, best-effort.
pub async fn record(state: &AppState, event: NewAuditEvent) {
    let action = event.action.clone();
    if let Err(err) = state.store.append_audit(event).await {
        tracing::error!(
            target: "audit",
            %err,
            action = %action,
            "audit.write_failed"
        );
    }
}

//! Best-effort audit-trail writes for security-sensitive mutations.
//!
//! An audit write must never break the primary operation it records — a failed
//! audit insert must not, for example, lose a freshly minted token secret that
//! only exists in the response body. So these writes are **best-effort**: on
//! error we emit a loud `tracing::error!` (greppable `audit.write_failed`) and
//! let the operation succeed; the store counts every failure
//! (`maidan_audit_write_failures_total`) and `MaidanAuditWriteFailures` pages
//! on the first.
//!
//! **Not the end state.** The maintainer decided (D-A, 2026-09-23) that
//! authority-changing actions write their audit row inside the change's own
//! transaction, so a failed write aborts the change; Cluster 413 moves them
//! there. Routine records stay on this best-effort path.
//!
//! Anonymous and ordinary denied requests (401/403) are deliberately *not*
//! written here — an attacker-controlled request stream would be an unbounded
//! audit-table write amplifier. Delegated denials are the exception: they come
//! from a named actor holding an expiring, revocable grant and are durably
//! recorded by the shared authorization lane.

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

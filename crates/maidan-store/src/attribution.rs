//! The principal behind whatever the current request writes.
//!
//! Every event is appended through one function per backend, deep below the
//! route that caused it, and threading a principal through each of the thirty-
//! odd `*_with_event` methods would be a rule the thirty-first forgets. The
//! server instead scopes each request once, and the append reads the scope.
//!
//! Absent a scope — scheduled sweeps, background workers, federation ingest —
//! nothing is recorded, which is what "the system did this" looks like. That is
//! safe because no request handler hands event writes to a spawned task: a
//! spawned task would leave the scope, and its writes would silently read as
//! the system's.

use maidan_types::Attribution;

tokio::task_local! {
    static ATTRIBUTION: Attribution;
}

/// Run `fut` with `attribution` recorded against everything it writes. `None`
/// runs it unscoped.
pub async fn with_attribution<F>(attribution: Option<Attribution>, fut: F) -> F::Output
where
    F: std::future::Future,
{
    match attribution {
        Some(attribution) => ATTRIBUTION.scope(attribution, fut).await,
        None => fut.await,
    }
}

/// The principal of the current request, if one is in scope.
pub fn current_attribution() -> Option<Attribution> {
    ATTRIBUTION.try_with(|attribution| *attribution).ok()
}

/// Record the current request's principal on an event payload, before it is
/// normalised, stored and hashed. Both backends call this at the same point: a
/// federated origin hash computed on one is verified on the other, so they must
/// produce byte-identical payloads for the same event.
pub(crate) fn attach_to_payload(
    payload: &mut serde_json::Value,
) -> Result<(), crate::error::StoreError> {
    if let (Some(attribution), Some(object)) = (current_attribution(), payload.as_object_mut()) {
        object.insert("attribution".into(), serde_json::to_value(attribution)?);
    }
    Ok(())
}

/// Who an audit row records: `(actor, subject, grant)`.
///
/// Inside a request the request's principal is authoritative and the caller's
/// `actor_id` is ignored, so no call site can misattribute — the fault this
/// replaces was twenty-five sites recording the member acted *for* as the one
/// who acted. Outside a request (a login before a principal exists, background
/// work) the caller names the actor, who acted for itself.
pub(crate) fn audit_principal(
    fallback_actor: Option<maidan_types::MemberId>,
) -> (
    Option<maidan_types::MemberId>,
    Option<maidan_types::MemberId>,
    Option<maidan_types::DelegationGrantId>,
) {
    match current_attribution() {
        Some(attribution) => (
            Some(attribution.actor_id),
            Some(attribution.subject_id),
            attribution.grant_id,
        ),
        None => (fallback_actor, fallback_actor, None),
    }
}

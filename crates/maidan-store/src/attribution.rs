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

use std::sync::atomic::{AtomicBool, Ordering};

use maidan_types::Attribution;

struct Scope {
    attribution: Attribution,
    /// Set by anything that writes an attributed record — an event or an audit
    /// row — so the layer that opened the scope can tell a request that changed
    /// state without leaving a record from one that left its own.
    recorded: AtomicBool,
}

tokio::task_local! {
    static SCOPE: Scope;
}

/// Run `fut` with `attribution` recorded against everything it writes. `None`
/// runs it unscoped.
pub async fn with_attribution<F>(attribution: Option<Attribution>, fut: F) -> F::Output
where
    F: std::future::Future,
{
    with_attribution_tracked(attribution, fut).await.0
}

/// [`with_attribution`], also reporting whether anything inside wrote an
/// attributed record. An unscoped run reports `false`.
pub async fn with_attribution_tracked<F>(
    attribution: Option<Attribution>,
    fut: F,
) -> (F::Output, bool)
where
    F: std::future::Future,
{
    match attribution {
        Some(attribution) => {
            let (out, recorded) = SCOPE
                .scope(
                    Scope {
                        attribution,
                        recorded: AtomicBool::new(false),
                    },
                    async {
                        let out = fut.await;
                        (out, SCOPE.with(|s| s.recorded.load(Ordering::Relaxed)))
                    },
                )
                .await;
            // A nested scope — a REST handler dispatching an MCP tool — records
            // for the request that encloses it, so the outer layer does not
            // write a second record of the same change.
            if recorded {
                mark_recorded();
            }
            (out, recorded)
        }
        None => (fut.await, false),
    }
}

/// The principal of the current request, if one is in scope.
pub fn current_attribution() -> Option<Attribution> {
    SCOPE.try_with(|scope| scope.attribution).ok()
}

/// The member actually acting in this request, when it is not `subject` — a
/// delegate using a token borrowed from `subject`. `None` for a member acting
/// for itself, for work outside a request, and for a request acting on some
/// *other* member (an orchestrator assigning a thread to a worker has not
/// worked it).
pub(crate) fn delegate_acting_for(
    subject: maidan_types::MemberId,
) -> Option<maidan_types::MemberId> {
    current_attribution()
        .filter(|a| a.subject_id == subject && a.actor_id != subject)
        .map(|a| a.actor_id)
}

fn mark_recorded() {
    let _ = SCOPE.try_with(|scope| scope.recorded.store(true, Ordering::Relaxed));
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
        mark_recorded();
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
        Some(attribution) => {
            mark_recorded();
            (
                Some(attribution.actor_id),
                Some(attribution.subject_id),
                attribution.grant_id,
            )
        }
        None => (fallback_actor, fallback_actor, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use maidan_types::MemberId;

    fn someone() -> Option<Attribution> {
        let member = MemberId(uuid::Uuid::new_v4());
        Some(Attribution {
            actor_id: member,
            subject_id: member,
            grant_id: None,
        })
    }

    #[tokio::test]
    async fn a_scope_reports_whether_anything_inside_it_was_recorded() {
        let ((), recorded) = with_attribution_tracked(someone(), async {}).await;
        assert!(!recorded);
        let (_, recorded) = with_attribution_tracked(someone(), async {
            attach_to_payload(&mut serde_json::json!({}))
        })
        .await;
        assert!(recorded);
    }

    /// A REST handler that dispatches an MCP tool opens a scope inside the
    /// request's. Whatever the tool records is recorded for the request too,
    /// or the request layer would write a second record of the same change.
    #[tokio::test]
    async fn a_record_inside_a_nested_scope_counts_for_the_enclosing_one() {
        let (_, outer) = with_attribution_tracked(someone(), async {
            with_attribution_tracked(someone(), async {
                attach_to_payload(&mut serde_json::json!({}))
            })
            .await
        })
        .await;
        assert!(outer);
    }
}

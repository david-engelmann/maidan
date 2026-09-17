//! Scheduled event-log chain verification.
//!
//! Whole-chain verification was moved out of the search tap's restart path, on
//! the argument that verification-by-restart is an accident rather
//! than a control — you cannot schedule it, alert on it, or say when it last
//! ran. That argument only holds if something *does* schedule it. This is that
//! thing.
//!
//! Opt-in, like the retention and scheduler sweepers: with
//! `MAIDAN_CHAIN_VERIFY_SECS` unset, nothing runs and an unconfigured
//! deployment is byte-unchanged. `verify_event_chain` streams rather than
//! materializing the whole log, which is what makes running it on a timer
//! affordable at all.

use std::sync::Arc;
use std::time::Duration;

use maidan_store::Store;

/// Read the sweep interval, or `None` when the verifier is not configured.
///
/// No default interval on purpose. A verifier that silently starts itself would
/// add a full-log read to every deployment that upgraded, which is exactly the
/// kind of surprise an operator should opt into.
pub fn interval_from_env() -> Option<Duration> {
    std::env::var("MAIDAN_CHAIN_VERIFY_SECS")
        .ok()
        .and_then(|s| s.trim().parse::<u64>().ok())
        .filter(|&s| s > 0)
        .map(Duration::from_secs)
}

/// Verify every workspace that has events. Returns `(checked, broken)`.
///
/// A break is reported per workspace and the sweep continues — one tenant's
/// broken chain must not stop the instance from learning about the others,
/// which is the same rule the search tap follows.
pub async fn sweep_once(store: &Arc<dyn Store>) -> (usize, usize) {
    let workspaces = match store.workspace_ids_with_events().await {
        Ok(v) => v,
        Err(err) => {
            tracing::warn!(error = %err, "chain verify: could not list workspaces");
            return (0, 0);
        }
    };
    let mut checked = 0usize;
    let mut broken = 0usize;
    for workspace_id in workspaces {
        match store.verify_event_chain(workspace_id).await {
            Ok(report) if report.ok => {
                checked += 1;
                crate::metrics::record_chain_verify("ok");
            }
            Ok(report) => {
                broken += 1;
                checked += 1;
                // Named, not counted. "A chain is broken" without saying which
                // room is the report that sends an operator reading everything.
                tracing::error!(
                    workspace_id = %workspace_id.0,
                    break_at = ?report.break_at,
                    reason = ?report.reason,
                    events_checked = report.checked,
                    "event-log chain verification FAILED for this workspace"
                );
                crate::metrics::record_chain_verify("broken");
            }
            Err(err) => {
                // Could not verify is not the same as verified-and-broken, and
                // collapsing them would let a database blip read as a tamper.
                tracing::warn!(
                    workspace_id = %workspace_id.0,
                    error = %err,
                    "chain verify: could not verify this workspace"
                );
                crate::metrics::record_chain_verify("error");
            }
        }
    }
    (checked, broken)
}

/// Sweep forever on `interval`.
pub async fn run(store: Arc<dyn Store>, interval: Duration) {
    tracing::info!(
        interval_secs = interval.as_secs(),
        "event-log chain verifier started"
    );
    loop {
        let (checked, broken) = sweep_once(&store).await;
        if broken > 0 {
            tracing::error!(checked, broken, "chain verification found broken chains");
        } else {
            tracing::debug!(checked, "chain verification passed");
        }
        tokio::time::sleep(interval).await;
    }
}

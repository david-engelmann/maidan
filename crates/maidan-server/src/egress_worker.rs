//! Background projector-egress worker (Cluster 377.2, durable projector egress).
//!
//! Drains the `maidan_egress_outbox` queue (Cluster 377.1): each tick claims due
//! `pending` deliveries and posts them through the configured projector sender —
//! [`SlackSender`](crate::slack::SlackSender) or
//! [`GithubSender`](crate::github::GithubSender) — marking each delivered, or, on
//! failure, rescheduled with exponential backoff, or dead-lettered once it has
//! exhausted [`MAX_ATTEMPTS`].
//!
//! Replaces the best-effort inline post the Slack (309) and GitHub (312)
//! projectors did, where a transient 502 dropped the message with a log line:
//! `route_message_to_slack` / `route_message_to_github` now only *enqueue*.
//!
//! **Runs whenever a projector sender is configured** (spawned in `main.rs` only
//! then — and the projectors only enqueue then, so an unconfigured deployment
//! neither queues nor drains). Tick defaults to 5s, tunable via
//! `MAIDAN_EGRESS_WORKER_TICK_SECS`.
//!
//! **At-least-once:** [`claim_next_due_egress`](maidan_store::Store) leases a row
//! forward, so a worker that crashes mid-post releases it after the lease and
//! another claim retries. A duplicate comment is the lesser harm against a
//! silently dropped one — the Cluster-255 digest polarity. Multiple replicas can
//! run the worker safely (`FOR UPDATE SKIP LOCKED` on Postgres hands each a
//! distinct row), and the queue's dedup index means they enqueue one row between
//! them in the first place.

use std::time::Duration;

use maidan_types::{EgressOutbox, EgressTarget};

use crate::state::AppState;

/// How far forward a claim leases a row. A projector post should finish well
/// within this; a crashed worker's row becomes re-claimable after it.
const LEASE_SECS: i64 = 120;

/// Attempts before a delivery is dead-lettered (the claim counts the current try,
/// so this bounds total posts per delivery).
const MAX_ATTEMPTS: i64 = 8;

/// Belt-and-suspenders bound on posts per tick, so a large backlog can't fire
/// unbounded API calls in one pass; the remainder drains on later ticks.
const MAX_PER_TICK: u32 = 1000;

const BACKOFF_BASE_SECS: u64 = 30;
const BACKOFF_CAP_SECS: u64 = 3600;

#[derive(Debug, Clone)]
pub struct EgressWorkerConfig {
    pub tick: Duration,
}

/// The default tick (5s), overridable via `MAIDAN_EGRESS_WORKER_TICK_SECS` (>0).
/// Like the mail worker, the egress worker is not opt-in by env — it is spawned
/// whenever a projector sender is configured — so this always returns a config.
pub fn config_from_env() -> EgressWorkerConfig {
    let secs = std::env::var("MAIDAN_EGRESS_WORKER_TICK_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|&s| s > 0)
        .unwrap_or(5);
    EgressWorkerConfig {
        tick: Duration::from_secs(secs),
    }
}

/// Exponential backoff for the n-th attempt (n counts the current claim, so the
/// first failure is `attempts == 1`): `base * 2^(n-1)`, capped.
fn backoff_for(attempts: i64) -> Duration {
    let exp = attempts.saturating_sub(1).clamp(0, 20) as u32;
    let secs = BACKOFF_BASE_SECS
        .saturating_mul(2u64.saturating_pow(exp))
        .min(BACKOFF_CAP_SECS);
    Duration::from_secs(secs)
}

/// Outcome tallies for a sweep (for tests / logging).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct EgressSweepStats {
    pub sent: u32,
    pub retried: u32,
    pub dead: u32,
}

/// Post one claimed delivery through the sender for its surface. `Err` carries
/// the message recorded as the row's `last_error`.
async fn deliver(state: &AppState, target: &EgressTarget, body: &str) -> Result<(), String> {
    match target {
        EgressTarget::Slack { channel_id } => {
            let sender = state
                .slack_sender
                .as_ref()
                .ok_or_else(|| "no slack sender configured".to_string())?;
            let result = sender.post_message(channel_id, body).await;
            crate::metrics::record_slack_egress(if result.is_ok() { "sent" } else { "failed" });
            result.map_err(|e| e.to_string())
        }
        EgressTarget::Github { repo, issue_number } => {
            let sender = state
                .github_sender
                .as_ref()
                .ok_or_else(|| "no github sender configured".to_string())?;
            let result = sender.post_comment(repo, *issue_number, body).await;
            crate::metrics::record_github_egress(if result.is_ok() { "sent" } else { "failed" });
            result.map_err(|e| e.to_string())
        }
    }
}

/// Record a failed delivery: reschedule with backoff, or dead-letter once the
/// attempts are exhausted.
async fn record_failure(state: &AppState, entry: &EgressOutbox, error: &str) -> bool {
    let dead = entry.attempts >= MAX_ATTEMPTS;
    let retry_at = (!dead).then(|| {
        chrono::Utc::now()
            + chrono::Duration::from_std(backoff_for(entry.attempts))
                .unwrap_or_else(|_| chrono::Duration::seconds(BACKOFF_BASE_SECS as i64))
    });
    if let Err(e) = state
        .store
        .mark_egress_failed(entry.id, error, retry_at)
        .await
    {
        tracing::warn!(error = %e, id = %entry.id, "egress worker: recording the failure failed");
    }
    if dead {
        tracing::warn!(
            error = %error,
            id = %entry.id,
            attempts = entry.attempts,
            surface = %entry.surface,
            "egress worker: dead-lettered"
        );
    }
    dead
}

/// Drain up to [`MAX_PER_TICK`] due deliveries. No-op when no projector sender is
/// configured — without one, every claim would fail and burn the queue's attempts
/// against a deployment that simply has the projector turned off.
pub async fn sweep_once(state: &AppState) -> EgressSweepStats {
    let mut stats = EgressSweepStats::default();
    if state.slack_sender.is_none() && state.github_sender.is_none() {
        return stats;
    }
    for _ in 0..MAX_PER_TICK {
        let now = chrono::Utc::now();
        let entry = match state.store.claim_next_due_egress(now, LEASE_SECS).await {
            Ok(Some(e)) => e,
            Ok(None) => break, // queue drained
            Err(err) => {
                tracing::warn!(error = %err, "egress worker: claim failed");
                break;
            }
        };
        // An undecodable destination can never be delivered by any sender, so it
        // dead-letters on the spot instead of burning eight attempts. This is why
        // the claim hands back the stored pair rather than decoding it itself.
        let Some(target) = entry.target() else {
            let error = format!(
                "unroutable destination: {}:{}",
                entry.surface, entry.selector
            );
            if let Err(e) = state.store.mark_egress_failed(entry.id, &error, None).await {
                tracing::warn!(error = %e, id = %entry.id, "egress worker: dead-letter failed");
            }
            tracing::warn!(id = %entry.id, %error, "egress worker: dead-lettered");
            stats.dead += 1;
            continue;
        };
        match deliver(state, &target, &entry.body).await {
            Ok(()) => {
                if let Err(err) = state.store.mark_egress_delivered(entry.id).await {
                    tracing::warn!(error = %err, id = %entry.id, "egress worker: mark-delivered failed");
                }
                stats.sent += 1;
            }
            Err(error) => {
                if record_failure(state, &entry, &error).await {
                    stats.dead += 1;
                } else {
                    stats.retried += 1;
                }
            }
        }
    }
    stats
}

/// Loop: sweep, then sleep `cfg.tick`. Spawned once at startup when a projector
/// sender is configured.
pub async fn run(state: AppState, cfg: EgressWorkerConfig) {
    tracing::info!(tick_secs = cfg.tick.as_secs(), "egress worker started");
    loop {
        sweep_once(&state).await;
        tokio::time::sleep(cfg.tick).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_then_caps() {
        assert_eq!(backoff_for(1), Duration::from_secs(30));
        assert_eq!(backoff_for(2), Duration::from_secs(60));
        assert_eq!(backoff_for(3), Duration::from_secs(120));
        // Caps at BACKOFF_CAP_SECS and never overflows for large attempt counts.
        assert_eq!(backoff_for(100), Duration::from_secs(BACKOFF_CAP_SECS));
    }
}

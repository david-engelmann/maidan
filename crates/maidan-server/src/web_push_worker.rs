//! Background web-push outbox worker.
//!
//! The notification router sends once inline. A failed send (not a gone
//! subscription) is queued in `maidan_web_push_outbox`. This worker claims due
//! rows and sends them again, with backoff, until they succeed, the
//! subscription is gone, or the attempt cap dead-letters the row.
//!
//! Runs when a VAPID sender is configured. Tick defaults to 5s
//! (`MAIDAN_WEBPUSH_WORKER_TICK_SECS`).

use std::time::Duration;

use chrono::{DateTime, Utc};

use crate::retry_budget::{deferred_until, host_of, Attempt};
use crate::state::AppState;
const LEASE_SECS: i64 = 120;
const MAX_ATTEMPTS: i64 = 8;
const MAX_PER_TICK: u32 = 1000;
const BACKOFF_BASE_SECS: u64 = 30;
const BACKOFF_CAP_SECS: u64 = 3600;

#[derive(Debug, Clone)]
pub struct WebPushWorkerConfig {
    pub tick: Duration,
}

pub fn config_from_env() -> WebPushWorkerConfig {
    let secs = std::env::var("MAIDAN_WEBPUSH_WORKER_TICK_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|&s| s > 0)
        .unwrap_or(5);
    WebPushWorkerConfig {
        tick: Duration::from_secs(secs),
    }
}

fn backoff_for(attempts: i64) -> Duration {
    let exp = attempts.saturating_sub(1).clamp(0, 20) as u32;
    let secs = BACKOFF_BASE_SECS
        .saturating_mul(2u64.saturating_pow(exp))
        .min(BACKOFF_CAP_SECS);
    Duration::from_secs(secs)
}

/// When a send that was attempt number `attempts` (1 = the first failure)
/// may be tried again.
pub fn next_attempt_at(attempts: i64) -> DateTime<Utc> {
    let delay = chrono::Duration::from_std(backoff_for(attempts))
        .unwrap_or_else(|_| chrono::Duration::seconds(BACKOFF_BASE_SECS as i64));
    Utc::now() + delay
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct WebPushSweepStats {
    pub sent: u32,
    pub retried: u32,
    pub dead: u32,
    pub pruned: u32,
    pub deferred: u32,
}

pub async fn sweep_once(state: &AppState) -> WebPushSweepStats {
    sweep_due(state, Utc::now()).await
}

/// Drain due rows as of `now`. Tests pass a later `now` so a backoff does not
/// have to elapse on the wall clock.
pub async fn sweep_due(state: &AppState, now: DateTime<Utc>) -> WebPushSweepStats {
    let Some(sender) = state.web_push.clone() else {
        return WebPushSweepStats::default();
    };
    let mut stats = WebPushSweepStats::default();
    for _ in 0..MAX_PER_TICK {
        let entry = match state.store.claim_next_due_web_push(now, LEASE_SECS).await {
            Ok(Some(entry)) => entry,
            Ok(None) => break,
            Err(err) => {
                tracing::warn!(error = %err, "web push worker: claim failed");
                break;
            }
        };
        let subs = match state.store.list_push_subscriptions(entry.member_id).await {
            Ok(subs) => subs,
            Err(err) => {
                tracing::warn!(error = %err, "web push worker: subscription lookup failed");
                let until = Utc::now() + chrono::Duration::seconds(BACKOFF_BASE_SECS as i64);
                if let Err(e) = state.store.defer_web_push(entry.id, until).await {
                    tracing::warn!(error = %e, id = %entry.id, "web push worker: deferral failed");
                }
                stats.deferred += 1;
                continue;
            }
        };
        let Some(sub) = subs.into_iter().find(|s| s.id == entry.subscription_id) else {
            if let Err(e) = state
                .store
                .mark_web_push_failed(entry.id, "subscription missing", None)
                .await
            {
                tracing::warn!(error = %e, id = %entry.id, "web push worker: drop missing subscription failed");
            }
            stats.dead += 1;
            continue;
        };
        let attempt = Attempt::after(entry.attempts - 1);
        if let Some(host) = host_of(&sub.endpoint) {
            if let Some(until) = deferred_until(&state.retry_budget, "web_push", &host, attempt) {
                if let Err(err) = state.store.defer_web_push(entry.id, until).await {
                    tracing::warn!(error = %err, id = %entry.id, "web push worker: deferral failed");
                }
                stats.deferred += 1;
                continue;
            }
        }
        match sender.send(&sub, entry.payload.as_bytes()).await {
            Ok(()) => {
                if let Err(err) = state.store.mark_web_push_delivered(entry.id).await {
                    tracing::warn!(error = %err, id = %entry.id, "web push worker: mark-delivered failed");
                }
                crate::metrics::record_web_push_delivered("sent");
                stats.sent += 1;
            }
            Err(err) if err.is_gone() => {
                crate::metrics::record_web_push_delivered("pruned");
                if let Err(e) = state
                    .store
                    .delete_push_subscription(entry.member_id, sub.id)
                    .await
                {
                    tracing::warn!(error = %e, "web push worker: pruning gone subscription failed");
                }
                if let Err(e) = state.store.mark_web_push_delivered(entry.id).await {
                    tracing::warn!(error = %e, id = %entry.id, "web push worker: mark pruned failed");
                }
                stats.pruned += 1;
            }
            Err(err) => {
                let msg = err.to_string();
                if entry.attempts >= MAX_ATTEMPTS {
                    if let Err(e) = state.store.mark_web_push_failed(entry.id, &msg, None).await {
                        tracing::warn!(error = %e, id = %entry.id, "web push worker: dead-letter failed");
                    }
                    tracing::warn!(error = %msg, id = %entry.id, attempts = entry.attempts, "web push worker: dead-lettered");
                    crate::metrics::record_web_push_delivered("dead");
                    stats.dead += 1;
                } else {
                    let retry_at = next_attempt_at(entry.attempts);
                    if let Err(e) = state
                        .store
                        .mark_web_push_failed(entry.id, &msg, Some(retry_at))
                        .await
                    {
                        tracing::warn!(error = %e, id = %entry.id, "web push worker: reschedule failed");
                    }
                    crate::metrics::record_web_push_delivered("retry");
                    stats.retried += 1;
                }
            }
        }
    }
    stats
}

pub async fn run(state: AppState, cfg: WebPushWorkerConfig) {
    tracing::info!(tick_secs = cfg.tick.as_secs(), "web push worker started");
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
        assert_eq!(backoff_for(100), Duration::from_secs(BACKOFF_CAP_SECS));
    }
}

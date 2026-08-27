//! Background mail-outbox worker (Cluster 305, durable mail retry queue).
//!
//! Drains the [`maidan_mail_outbox`](crate::mcp) queue (Cluster 304): each tick
//! claims due `pending` entries and sends them via the configured
//! [`MailTransport`](crate::mail::MailTransport), marking each delivered, or —
//! on failure — rescheduled with exponential backoff, or dead-lettered once it
//! has exhausted [`MAX_ATTEMPTS`].
//!
//! Replaces the best-effort fire-and-forget send the notification router used to
//! do inline (Cluster 249): the router now only *enqueues*, so a transient SMTP
//! failure is retried instead of dropped.
//!
//! **Runs whenever a transport is configured** (spawned in `main.rs` only when
//! `state.mail` is set — a queue with no sender would just pile up). Tick defaults
//! to 5s, tunable via `MAIDAN_MAIL_WORKER_TICK_SECS`.
//!
//! **At-least-once:** [`Store::claim_next_due_mail`](maidan_store::Store) leases a
//! row forward, so a worker that crashes mid-send releases it after the lease and
//! another claim retries — a duplicate email is low-harm (the Cluster-255 digest
//! polarity). Multiple replicas can run the worker safely (`FOR UPDATE SKIP
//! LOCKED` on Postgres hands each a distinct row).

use std::time::Duration;

use crate::state::AppState;

/// How far forward a claim leases a row (a send attempt should finish well within
/// this; a crashed worker's row becomes re-claimable after it).
const LEASE_SECS: i64 = 120;

/// Attempts before a message is dead-lettered (the claim counts the current try,
/// so this bounds total sends per message).
const MAX_ATTEMPTS: i64 = 8;

/// Belt-and-suspenders bound on sends per tick, so a large backlog can't send
/// unbounded emails in one pass; the remainder drains on later ticks.
const MAX_PER_TICK: u32 = 1000;

const BACKOFF_BASE_SECS: u64 = 30;
const BACKOFF_CAP_SECS: u64 = 3600;

#[derive(Debug, Clone)]
pub struct MailWorkerConfig {
    pub tick: Duration,
}

/// The default tick (5s), overridable via `MAIDAN_MAIL_WORKER_TICK_SECS` (>0).
/// Unlike the digest sweeper, the worker is not opt-in by env — it's spawned
/// whenever a transport is configured — so this always returns a config.
pub fn config_from_env() -> MailWorkerConfig {
    let secs = std::env::var("MAIDAN_MAIL_WORKER_TICK_SECS")
        .ok()
        .and_then(|v| v.trim().parse::<u64>().ok())
        .filter(|&s| s > 0)
        .unwrap_or(5);
    MailWorkerConfig {
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
pub struct MailSweepStats {
    pub sent: u32,
    pub retried: u32,
    pub dead: u32,
}

/// Drain up to [`MAX_PER_TICK`] due entries, sending each. No-op without a
/// transport.
pub async fn sweep_once(state: &AppState) -> MailSweepStats {
    let Some(mail) = state.mail.clone() else {
        return MailSweepStats::default();
    };
    let mut stats = MailSweepStats::default();
    for _ in 0..MAX_PER_TICK {
        let now = chrono::Utc::now();
        let entry = match state.store.claim_next_due_mail(now, LEASE_SECS).await {
            Ok(Some(e)) => e,
            Ok(None) => break, // queue drained
            Err(err) => {
                tracing::warn!(error = %err, "mail worker: claim failed");
                break;
            }
        };
        match mail
            .send(&entry.to_address, &entry.subject, &entry.body)
            .await
        {
            Ok(()) => {
                if let Err(err) = state.store.mark_mail_delivered(entry.id).await {
                    tracing::warn!(error = %err, id = %entry.id, "mail worker: mark-delivered failed");
                }
                crate::metrics::record_email_delivered("sent");
                stats.sent += 1;
            }
            Err(err) => {
                let msg = err.to_string();
                if entry.attempts >= MAX_ATTEMPTS {
                    if let Err(e) = state.store.mark_mail_failed(entry.id, &msg, None).await {
                        tracing::warn!(error = %e, id = %entry.id, "mail worker: dead-letter failed");
                    }
                    tracing::warn!(error = %msg, id = %entry.id, attempts = entry.attempts, "mail worker: dead-lettered");
                    crate::metrics::record_email_delivered("dead");
                    stats.dead += 1;
                } else {
                    let retry_at = chrono::Utc::now()
                        + chrono::Duration::from_std(backoff_for(entry.attempts)).unwrap_or_else(
                            |_| chrono::Duration::seconds(BACKOFF_BASE_SECS as i64),
                        );
                    if let Err(e) = state
                        .store
                        .mark_mail_failed(entry.id, &msg, Some(retry_at))
                        .await
                    {
                        tracing::warn!(error = %e, id = %entry.id, "mail worker: reschedule failed");
                    }
                    crate::metrics::record_email_delivered("retry");
                    stats.retried += 1;
                }
            }
        }
    }
    stats
}

/// Loop: sweep, then sleep `cfg.tick`. Spawned once at startup when a transport
/// is configured.
pub async fn run(state: AppState, cfg: MailWorkerConfig) {
    tracing::info!(tick_secs = cfg.tick.as_secs(), "mail worker started");
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

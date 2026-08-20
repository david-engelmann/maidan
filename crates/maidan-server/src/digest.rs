//! Background email-digest sweeper (Cluster 255, Program C — Arc I).
//!
//! Opt-in via `MAIDAN_DIGEST_TICK_SECS` (>0). Each tick drains
//! [`Store::members_due_for_digest`] — members in `Digest` delivery mode
//! (Cluster 254) who have an address on file and unread notifications created
//! since their last digest — and emails each an unread-count rollup, then
//! advances their digest watermark ([`Store::set_last_digest_at`]).
//!
//! **At-least-once, self-healing:** the watermark is advanced only after a
//! successful send, so a transient SMTP failure simply retries on the next tick
//! rather than dropping the digest. A crash between send and advance re-sends
//! (a duplicate digest, not a lost one) — the right trade for a rollup email.
//!
//! **Multi-replica note:** the claim (`members_due_for_digest`) and the advance
//! (`set_last_digest_at`) are not a single atomic step, so two replicas both
//! running the sweeper could double-send a digest before either advances the
//! watermark. A duplicate digest is low-harm (unlike a duplicate task thread),
//! so — deliberately, unlike the Cluster-227 scheduler's `SKIP LOCKED` claim —
//! the sweeper does not single-flight; run it on one replica (the common cron
//! deployment) if exactly-once digests matter.
//!
//! Does nothing when no `MailTransport` is configured (a digest with no way to
//! send is a no-op).

use std::time::Duration;

use crate::state::AppState;

/// Belt-and-suspenders bound on digests sent per tick, so a large backlog can't
/// send unbounded emails in one pass; the remainder goes out on later ticks.
const MAX_DIGESTS_PER_TICK: i64 = 1000;

#[derive(Debug, Clone)]
pub struct DigestConfig {
    pub tick: Duration,
}

/// Build the config from the environment, or `None` when `MAIDAN_DIGEST_TICK_SECS`
/// is unset / non-positive (the sweeper is not started — digests never go out
/// until an operator enables it).
pub fn config_from_env() -> Option<DigestConfig> {
    let secs = std::env::var("MAIDAN_DIGEST_TICK_SECS")
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()
        .filter(|&s| s > 0)?;
    Some(DigestConfig {
        tick: Duration::from_secs(secs),
    })
}

fn digest_subject() -> String {
    "Your Maidan notification digest".to_string()
}

fn digest_body(unread_count: i64) -> String {
    let noun = if unread_count == 1 {
        "notification"
    } else {
        "notifications"
    };
    format!("You have {unread_count} unread {noun} in Maidan. Open Maidan to catch up.")
}

/// Send a digest to every member currently due, advancing each watermark on a
/// successful send. Returns the number of digests sent (for tests / logging).
/// No-op when no mail transport is configured.
pub async fn sweep_once(state: &AppState) -> u32 {
    let Some(mail) = state.mail.clone() else {
        return 0;
    };
    let due = match state
        .store
        .members_due_for_digest(MAX_DIGESTS_PER_TICK)
        .await
    {
        Ok(d) => d,
        Err(err) => {
            tracing::warn!(error = %err, "digest sweeper: enumeration failed");
            return 0;
        }
    };
    let now = chrono::Utc::now();
    let mut sent = 0u32;
    for member in due {
        let subject = digest_subject();
        let body = digest_body(member.unread_count);
        match mail.send(&member.email, &subject, &body).await {
            Ok(()) => {
                // Advance the watermark only on success, so a failed send retries
                // next tick rather than silently dropping the digest.
                if let Err(err) = state.store.set_last_digest_at(member.member_id, now).await {
                    tracing::warn!(
                        error = %err,
                        member = %member.member_id,
                        "digest sweeper: watermark advance failed (will re-send next tick)"
                    );
                    continue;
                }
                crate::metrics::record_email_delivered("digest");
                sent += 1;
            }
            Err(err) => {
                tracing::warn!(error = %err, member = %member.member_id, "digest send failed");
                crate::metrics::record_email_delivered("digest_failed");
            }
        }
    }
    sent
}

/// Loop: sweep, then sleep `cfg.tick`. Spawned once at startup when configured.
pub async fn run(state: AppState, cfg: DigestConfig) {
    tracing::info!(tick_secs = cfg.tick.as_secs(), "digest sweeper started");
    loop {
        sweep_once(&state).await;
        tokio::time::sleep(cfg.tick).await;
    }
}

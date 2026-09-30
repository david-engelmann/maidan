//! The claim reaper: a lapsed lease is returned to the queue on time.
//!
//! Without it, a claim whose holder died was freed only when someone next
//! called `claim_next` on that channel, and `ClaimExpired` fired then — on an
//! idle channel, never. The reaper runs on every replica, on by default
//! (`MAIDAN_CLAIM_REAP_TICK_SECS`, 5 s; `0` turns it off). Each tick it takes
//! lapsed leases in batches (`Store::reap_expired_claims`, `SKIP LOCKED` on
//! Postgres so replicas split the work), clears the holder and its fencing
//! token, and publishes the `ClaimExpired` the store appended in the same
//! transaction, which the notification router turns into a stuck-work notice.
//!
//! `claim_next` still reclaims a lease that lapsed between ticks; a lease is
//! reported once either way, because whichever takes it clears the holder.
//!
//! The same tick reports claims their holder never acknowledged: a leased
//! claim still unacknowledged `MAIDAN_CLAIM_ACK_TIMEOUT_SECS` (120 s; `0`
//! turns it off) after it was taken gets one `ClaimUnacknowledged`, the push
//! for an agent that crashed right after claiming or never started. The
//! claim is left alone; its lease decides when the thread comes back.

use std::time::Duration;

use metrics::counter;

use crate::state::AppState;

/// Default seconds between reaper ticks.
pub const DEFAULT_TICK_SECS: u64 = 5;

/// Leases reaped per store call.
const BATCH: i64 = 100;

/// Upper bound on leases reaped in one tick, so a large backlog cannot hold
/// a tick open; the rest go on the next one.
const MAX_PER_TICK: usize = 1000;

/// Default seconds a leased claim may stay unacknowledged before
/// `ClaimUnacknowledged` fires.
pub const DEFAULT_ACK_TIMEOUT_SECS: u64 = 120;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimReaperConfig {
    pub tick: Duration,
    /// How long a leased claim may go unacknowledged; `None` reports nothing.
    pub ack_timeout: Option<Duration>,
}

/// Read `MAIDAN_CLAIM_REAP_TICK_SECS` and `MAIDAN_CLAIM_ACK_TIMEOUT_SECS`.
/// An unset or unparsable tick keeps the 5 s default and `0` turns the
/// reaper off (`None`); an unset or unparsable timeout keeps 120 s and `0`
/// turns the unacknowledged report off.
pub fn config_from_env() -> Option<ClaimReaperConfig> {
    config_from_raw(
        std::env::var("MAIDAN_CLAIM_REAP_TICK_SECS").ok(),
        std::env::var("MAIDAN_CLAIM_ACK_TIMEOUT_SECS").ok(),
    )
}

fn secs_or(raw: Option<String>, default: u64) -> u64 {
    raw.and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(default)
}

fn config_from_raw(tick: Option<String>, ack_timeout: Option<String>) -> Option<ClaimReaperConfig> {
    let tick = secs_or(tick, DEFAULT_TICK_SECS);
    let ack_timeout = secs_or(ack_timeout, DEFAULT_ACK_TIMEOUT_SECS);
    (tick > 0).then(|| ClaimReaperConfig {
        tick: Duration::from_secs(tick),
        ack_timeout: (ack_timeout > 0).then(|| Duration::from_secs(ack_timeout)),
    })
}

/// Reap every lease that lapsed before now, up to [`MAX_PER_TICK`], and
/// publish each `ClaimExpired`. Returns how many were reaped.
pub async fn sweep_once(state: &AppState) -> usize {
    let now = chrono::Utc::now();
    let mut reaped = 0;
    while reaped < MAX_PER_TICK {
        let events = match state.store.reap_expired_claims(now, BATCH).await {
            Ok(events) => events,
            Err(err) => {
                tracing::warn!(error = %err, "claim_reaper.reap_failed");
                break;
            }
        };
        let batch = events.len();
        for stored in events {
            crate::routes::publish_stored(state, stored).await;
        }
        reaped += batch;
        counter!("maidan_claims_reaped_total").increment(batch as u64);
        if batch < BATCH as usize {
            break;
        }
    }
    reaped
}

/// Report every leased claim taken more than `ack_timeout` ago that its
/// holder has not acknowledged, up to [`MAX_PER_TICK`], and publish each
/// `ClaimUnacknowledged`. Returns how many were reported.
pub async fn report_unacknowledged_once(state: &AppState, ack_timeout: Duration) -> usize {
    let now = chrono::Utc::now();
    let Ok(window) = chrono::Duration::from_std(ack_timeout) else {
        return 0;
    };
    let claimed_before = now - window;
    let mut reported = 0;
    while reported < MAX_PER_TICK {
        let events = match state
            .store
            .report_unacknowledged_claims(now, claimed_before, BATCH)
            .await
        {
            Ok(events) => events,
            Err(err) => {
                tracing::warn!(error = %err, "claim_reaper.unacknowledged_failed");
                break;
            }
        };
        let batch = events.len();
        for stored in events {
            crate::routes::publish_stored(state, stored).await;
        }
        reported += batch;
        counter!("maidan_claims_unacknowledged_total").increment(batch as u64);
        if batch < BATCH as usize {
            break;
        }
    }
    reported
}

/// The reaper loop, spawned by `main.rs` unless turned off.
pub async fn run(state: AppState, cfg: ClaimReaperConfig) {
    tracing::info!(
        tick_secs = cfg.tick.as_secs(),
        ack_timeout_secs = cfg.ack_timeout.map(|t| t.as_secs()),
        "claim reaper started"
    );
    loop {
        sweep_once(&state).await;
        if let Some(ack_timeout) = cfg.ack_timeout {
            report_unacknowledged_once(&state, ack_timeout).await;
        }
        tokio::time::sleep(cfg.tick).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reaper_is_on_by_default_and_zero_turns_it_off() {
        let default_ack = Some(Duration::from_secs(DEFAULT_ACK_TIMEOUT_SECS));
        assert_eq!(
            config_from_raw(None, None),
            Some(ClaimReaperConfig {
                tick: Duration::from_secs(DEFAULT_TICK_SECS),
                ack_timeout: default_ack,
            })
        );
        assert_eq!(
            config_from_raw(Some("garbled".into()), None),
            Some(ClaimReaperConfig {
                tick: Duration::from_secs(DEFAULT_TICK_SECS),
                ack_timeout: default_ack,
            })
        );
        assert_eq!(
            config_from_raw(Some(" 30 ".into()), None),
            Some(ClaimReaperConfig {
                tick: Duration::from_secs(30),
                ack_timeout: default_ack,
            })
        );
        assert_eq!(config_from_raw(Some("0".into()), None), None);
    }

    #[test]
    fn the_unacknowledged_report_has_its_own_switch() {
        let cfg = |raw: &str| config_from_raw(None, Some(raw.into())).map(|c| c.ack_timeout);
        assert_eq!(cfg("300"), Some(Some(Duration::from_secs(300))));
        assert_eq!(
            cfg("0"),
            Some(None),
            "0 turns the report off, not the reaper"
        );
        assert_eq!(
            cfg("soon"),
            Some(Some(Duration::from_secs(DEFAULT_ACK_TIMEOUT_SECS)))
        );
    }
}

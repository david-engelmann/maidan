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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimReaperConfig {
    pub tick: Duration,
}

/// Read `MAIDAN_CLAIM_REAP_TICK_SECS`: unset or unparsable keeps the 5 s
/// default, `0` turns the reaper off (`None`).
pub fn config_from_env() -> Option<ClaimReaperConfig> {
    config_from_raw(std::env::var("MAIDAN_CLAIM_REAP_TICK_SECS").ok())
}

fn config_from_raw(raw: Option<String>) -> Option<ClaimReaperConfig> {
    let secs = raw
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_TICK_SECS);
    (secs > 0).then(|| ClaimReaperConfig {
        tick: Duration::from_secs(secs),
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

/// The reaper loop, spawned by `main.rs` unless turned off.
pub async fn run(state: AppState, cfg: ClaimReaperConfig) {
    tracing::info!(tick_secs = cfg.tick.as_secs(), "claim reaper started");
    loop {
        sweep_once(&state).await;
        tokio::time::sleep(cfg.tick).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_reaper_is_on_by_default_and_zero_turns_it_off() {
        assert_eq!(
            config_from_raw(None),
            Some(ClaimReaperConfig {
                tick: Duration::from_secs(DEFAULT_TICK_SECS)
            })
        );
        assert_eq!(
            config_from_raw(Some("garbled".into())),
            Some(ClaimReaperConfig {
                tick: Duration::from_secs(DEFAULT_TICK_SECS)
            })
        );
        assert_eq!(
            config_from_raw(Some(" 30 ".into())),
            Some(ClaimReaperConfig {
                tick: Duration::from_secs(30)
            })
        );
        assert_eq!(config_from_raw(Some("0".into())), None);
    }
}

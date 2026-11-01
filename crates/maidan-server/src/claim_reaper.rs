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
//! The same transaction charges the time the claim worked, from its
//! acknowledgement to its lease deadline, to the thread's wall budget: a hung
//! agent never calls `report_usage`, so without the charge `max_wall_secs`
//! never bound it and the thread went round the queue forever. A claim that
//! leaves its thread over budget ends the way a report over budget ends it,
//! with `ClaimFailed` and a DLQ entry in place of `ClaimExpired`.
//!
//! `claim_next` still reclaims a lease that lapsed between ticks, and charges
//! it the same way; a lease is reported and charged once either way, because
//! whichever takes it clears the holder.
//!
//! The same tick reports claims their holder never acknowledged: a leased
//! claim still unacknowledged `MAIDAN_CLAIM_ACK_TIMEOUT_SECS` (120 s; `0`
//! turns it off) after it was taken gets one `ClaimUnacknowledged`, the push
//! for an agent that crashed right after claiming or never started. The
//! claim is left alone; its lease decides when the thread comes back.
//!
//! It also reports a live claim that looks stuck before the lease runs out,
//! as `ClaimStalled`, once per claim per reason and again leaving the claim
//! alone. `no_progress`: the holder acknowledged but has shown no progress
//! for `MAIDAN_STALL_SECS`, the same policy that marks a board card
//! `stalled`, so a hung agent whose heartbeat still renews is pushed to its
//! owner rather than only shown. `lease_lapsing`: the lease lapses within
//! `MAIDAN_CLAIM_LAPSE_WARN_SECS` (60 s; `0` turns it off), the pre-warning
//! before this reaper takes the claim back.

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

/// Default seconds before a lease lapses that `ClaimStalled`
/// (`lease_lapsing`) warns.
pub const DEFAULT_LAPSE_WARN_SECS: u64 = 60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimReaperConfig {
    pub tick: Duration,
    /// How long a leased claim may go unacknowledged; `None` reports nothing.
    pub ack_timeout: Option<Duration>,
    /// How long a holder may wait on a person before its thread goes back to
    /// the queue (`crate::human_wait`); `None` returns nothing.
    pub human_timeout: Option<Duration>,
    /// How long before a lease lapses its holder's owner is warned; `None`
    /// warns nobody.
    pub lapse_warn: Option<Duration>,
    /// How long a holder may show no progress before its claim is reported
    /// stalled (`MAIDAN_STALL_SECS`, [`crate::stall`]); `None` reports
    /// nothing.
    pub stall_threshold: Option<Duration>,
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
    .map(|cfg| ClaimReaperConfig {
        human_timeout: crate::human_wait::timeout_from_env(),
        lapse_warn: lapse_warn_from_raw(std::env::var("MAIDAN_CLAIM_LAPSE_WARN_SECS").ok()),
        stall_threshold: crate::stall::threshold_from_env(),
        ..cfg
    })
}

/// `MAIDAN_CLAIM_LAPSE_WARN_SECS`: unset or unparsable keeps 60 s, `0` turns
/// the warning off.
fn lapse_warn_from_raw(raw: Option<String>) -> Option<Duration> {
    let secs = secs_or(raw, DEFAULT_LAPSE_WARN_SECS);
    (secs > 0).then(|| Duration::from_secs(secs))
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
        human_timeout: None,
        lapse_warn: None,
        stall_threshold: None,
    })
}

/// Reap every lease that lapsed before now, up to [`MAX_PER_TICK`], and
/// publish each `ClaimExpired` (or `ClaimFailed`, for a claim that ran its
/// thread out of budget). Returns how many were reaped.
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

/// Report every live claim due a `ClaimStalled`, up to [`MAX_PER_TICK`] per
/// reason, and publish each. `stall_threshold` drives `no_progress` and
/// `lapse_warn` drives `lease_lapsing`; `None` skips that reason. Returns how
/// many were reported.
pub async fn report_stalled_once(
    state: &AppState,
    stall_threshold: Option<Duration>,
    lapse_warn: Option<Duration>,
) -> usize {
    use maidan_types::ClaimStallReason;
    let now = chrono::Utc::now();
    let mut reported = 0;
    for (reason, window) in [
        (ClaimStallReason::NoProgress, stall_threshold),
        (ClaimStallReason::LeaseLapsing, lapse_warn),
    ] {
        let Some(window) = window.and_then(|w| chrono::Duration::from_std(w).ok()) else {
            continue;
        };
        let cutoff = match reason {
            ClaimStallReason::NoProgress => now.checked_sub_signed(window),
            ClaimStallReason::LeaseLapsing => now.checked_add_signed(window),
        };
        // Past chrono's range the deadline never comes: nothing is due.
        let Some(cutoff) = cutoff else {
            continue;
        };
        let mut for_reason = 0;
        while for_reason < MAX_PER_TICK {
            let events = match state
                .store
                .report_stalled_claims(now, reason, cutoff, BATCH)
                .await
            {
                Ok(events) => events,
                Err(err) => {
                    tracing::warn!(error = %err, reason = reason.as_str(), "claim_reaper.stalled_failed");
                    break;
                }
            };
            let batch = events.len();
            for stored in events {
                crate::routes::publish_stored(state, stored).await;
            }
            for_reason += batch;
            counter!("maidan_claims_stalled_total", "reason" => reason.as_str())
                .increment(batch as u64);
            if batch < BATCH as usize {
                break;
            }
        }
        reported += for_reason;
    }
    reported
}

/// The reaper loop, spawned by `main.rs` unless turned off.
pub async fn run(state: AppState, cfg: ClaimReaperConfig) {
    tracing::info!(
        tick_secs = cfg.tick.as_secs(),
        ack_timeout_secs = cfg.ack_timeout.map(|t| t.as_secs()),
        human_timeout_secs = cfg.human_timeout.map(|t| t.as_secs()),
        lapse_warn_secs = cfg.lapse_warn.map(|t| t.as_secs()),
        stall_secs = cfg.stall_threshold.map(|t| t.as_secs()),
        "claim reaper started"
    );
    let _watched = crate::ops_alert::start_loop("claim_reaper", cfg.tick);
    loop {
        crate::ops_alert::guarded_tick("claim_reaper", async {
            sweep_once(&state).await;
            if let Some(ack_timeout) = cfg.ack_timeout {
                report_unacknowledged_once(&state, ack_timeout).await;
            }
            report_stalled_once(&state, cfg.stall_threshold, cfg.lapse_warn).await;
            // The same tick, because returning a thread to the queue is this
            // loop's job whether the holder died or is waiting on a silence.
            if let Some(human_timeout) = cfg.human_timeout {
                crate::human_wait::return_timed_out_once(&state, human_timeout).await;
            }
        })
        .await;
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
                human_timeout: None,
                lapse_warn: None,
                stall_threshold: None,
            })
        );
        assert_eq!(
            config_from_raw(Some("garbled".into()), None),
            Some(ClaimReaperConfig {
                tick: Duration::from_secs(DEFAULT_TICK_SECS),
                ack_timeout: default_ack,
                human_timeout: None,
                lapse_warn: None,
                stall_threshold: None,
            })
        );
        assert_eq!(
            config_from_raw(Some(" 30 ".into()), None),
            Some(ClaimReaperConfig {
                tick: Duration::from_secs(30),
                ack_timeout: default_ack,
                human_timeout: None,
                lapse_warn: None,
                stall_threshold: None,
            })
        );
        assert_eq!(config_from_raw(Some("0".into()), None), None);
    }

    #[test]
    fn the_lapse_warning_defaults_to_a_minute_and_zero_turns_it_off() {
        assert_eq!(
            lapse_warn_from_raw(None),
            Some(Duration::from_secs(DEFAULT_LAPSE_WARN_SECS))
        );
        assert_eq!(
            lapse_warn_from_raw(Some("garbled".into())),
            Some(Duration::from_secs(DEFAULT_LAPSE_WARN_SECS))
        );
        assert_eq!(
            lapse_warn_from_raw(Some(" 90 ".into())),
            Some(Duration::from_secs(90))
        );
        assert_eq!(lapse_warn_from_raw(Some("0".into())), None);
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

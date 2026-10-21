//! Operator alerts: a background loop that panics or stops ticking says so in
//! one Slack channel (`MAIDAN_OPS_ALERT_CHANNEL`, a channel id such as
//! `C0123ABCD`), through the Slack sender result delivery already uses.
//!
//! A local stack runs no Prometheus or Alertmanager, so the change sweep's
//! hour-long outage on 2026-10-20 reached nobody (#1476). The metrics and
//! rules in `docs/alerts/` serve a stack that scrapes; this serves one that
//! does not. It is an operator setting, like the Slack token itself: the
//! channel is named by whoever runs the server, not by a workspace.
//!
//! On boot the server posts one line, so every deploy shows the route works
//! end to end. A panicking tick posts at once. A watchdog posts when a loop
//! has not finished a tick for three of its intervals. The same alert is
//! posted at most once every [`REPEAT_SECS`], so a loop that panics every
//! tick does not flood the channel.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use crate::state::AppState;

/// The same alert, at most once in this many seconds.
pub const REPEAT_SECS: i64 = 900;

/// How often the watchdog looks.
const WATCHDOG_SECS: u64 = 60;

/// A loop is stalled after this many of its intervals without a tick.
const STALL_INTERVALS: i64 = 3;

static CHANNEL: OnceLock<String> = OnceLock::new();

/// Each registered loop's last tick (unix seconds) and its interval.
static LOOPS: OnceLock<Mutex<HashMap<&'static str, (i64, i64)>>> = OnceLock::new();

/// When each alert key last posted (unix seconds).
static SENT: OnceLock<Mutex<HashMap<String, i64>>> = OnceLock::new();

fn loops() -> &'static Mutex<HashMap<&'static str, (i64, i64)>> {
    LOOPS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn sent() -> &'static Mutex<HashMap<String, i64>> {
    SENT.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Read `MAIDAN_OPS_ALERT_CHANNEL`: a Slack channel id (`C…` or `G…`).
/// Unset, blank or not an id is off.
pub fn channel_from_env() -> Option<String> {
    channel_from_raw(std::env::var("MAIDAN_OPS_ALERT_CHANNEL").ok())
}

fn channel_from_raw(raw: Option<String>) -> Option<String> {
    let channel = raw?.trim().to_string();
    let id = channel.len() >= 9
        && channel.starts_with(['C', 'G'])
        && channel
            .chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit());
    if !id {
        tracing::warn!(
            value = channel,
            "MAIDAN_OPS_ALERT_CHANNEL is not a Slack channel id; operator alerts are off"
        );
        return None;
    }
    Some(channel)
}

/// A background loop started, with its interval: the watchdog now expects a
/// tick from it every interval.
pub fn register_loop(name: &'static str, interval: Duration) {
    let interval = i64::try_from(interval.as_secs()).unwrap_or(i64::MAX);
    if let Ok(mut loops) = loops().lock() {
        loops.insert(name, (now(), interval));
    }
}

/// A registered loop finished a tick.
pub fn note_tick(name: &'static str) {
    if let Ok(mut loops) = loops().lock() {
        if let Some(entry) = loops.get_mut(name) {
            entry.0 = now();
        }
    }
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// Whether a loop that last ticked at `last`, every `interval` seconds, is
/// stalled at `at`.
fn stalled(at: i64, last: i64, interval: i64) -> bool {
    at - last > interval.saturating_mul(STALL_INTERVALS)
}

/// Whether `key` may post at `at`, recording it when it may.
fn may_send(log: &mut HashMap<String, i64>, key: &str, at: i64) -> bool {
    match log.get(key) {
        Some(last) if at - last < REPEAT_SECS => false,
        _ => {
            log.insert(key.to_string(), at);
            true
        }
    }
}

/// Post `text` to the operator channel, unless the same `key` posted in the
/// last [`REPEAT_SECS`]. A no-op when alerts are off. Best effort: a failed
/// post is logged at WARN.
pub async fn alert(state: &AppState, key: &str, text: &str) {
    let (Some(channel), Some(sender)) = (CHANNEL.get(), state.slack_sender.as_ref()) else {
        return;
    };
    let allowed = sent()
        .lock()
        .map(|mut log| may_send(&mut log, key, now()))
        .unwrap_or(true);
    if !allowed {
        return;
    }
    if let Err(err) = sender.post_message(channel, text, None).await {
        tracing::warn!(error = %err, key, "operator alert not posted");
    }
}

/// Turn operator alerts on: post the boot line, then watch the registered
/// loops for stalls. Spawned by `main.rs` when the channel is set.
pub async fn run(state: AppState, channel: String) {
    if CHANNEL.set(channel).is_err() {
        return;
    }
    alert(
        &state,
        "boot",
        &format!(
            ":white_check_mark: Maidan {} started; operator alerts reach this channel.",
            env!("CARGO_PKG_VERSION")
        ),
    )
    .await;
    loop {
        tokio::time::sleep(Duration::from_secs(WATCHDOG_SECS)).await;
        let at = now();
        let late: Vec<(&'static str, i64)> = loops()
            .lock()
            .map(|loops| {
                loops
                    .iter()
                    .filter(|(_, (last, interval))| stalled(at, *last, *interval))
                    .map(|(name, (last, _))| (*name, at - last))
                    .collect()
            })
            .unwrap_or_default();
        for (name, quiet) in late {
            alert(
                &state,
                &format!("stall:{name}"),
                &format!(
                    ":rotating_light: Maidan's {name} loop has not finished a tick in {} min. Work it drives has stopped; check the maidan-server log.",
                    quiet / 60
                ),
            )
            .await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_channel_id_turns_alerts_on() {
        assert_eq!(
            channel_from_raw(Some(" C0123ABCDE ".into())).as_deref(),
            Some("C0123ABCDE")
        );
        assert!(channel_from_raw(Some("G01234567".into())).is_some());
        for raw in ["#ops-alerts", "ops-alerts", "C01", "c0123abcde", ""] {
            assert!(channel_from_raw(Some(raw.into())).is_none(), "{raw}");
        }
        assert!(channel_from_raw(None).is_none());
    }

    #[test]
    fn a_loop_is_stalled_after_three_intervals() {
        assert!(!stalled(1_000, 1_000 - 360, 120));
        assert!(stalled(1_000, 1_000 - 361, 120));
    }

    #[test]
    fn the_same_alert_posts_once_per_window() {
        let mut log = HashMap::new();
        assert!(may_send(&mut log, "stall:change_sweep", 0));
        assert!(!may_send(&mut log, "stall:change_sweep", REPEAT_SECS - 1));
        assert!(
            may_send(&mut log, "panic:change_sweep", 1),
            "another key is its own"
        );
        assert!(may_send(&mut log, "stall:change_sweep", REPEAT_SECS));
    }
}

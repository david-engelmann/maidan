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
//!
//! Every background loop goes through here: [`start_loop`] when it starts,
//! [`guarded_tick`] around each tick (a panic is counted, logged, alerted,
//! and the loop runs on), and [`stop_loop`] if it ends on purpose. The
//! liveness gauge and panic counter (`metrics.rs`) feed the Prometheus
//! rules; this feeds the Slack channel.

use std::collections::HashMap;
use std::future::Future;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use crate::slack::SlackSender;
use crate::state::AppState;

/// The same alert, at most once in this many seconds.
pub const REPEAT_SECS: i64 = 900;

/// How often the watchdog looks.
const WATCHDOG_SECS: u64 = 60;

/// A loop is stalled after this many of its intervals without a tick.
const STALL_INTERVALS: i64 = 3;

static CHANNEL: OnceLock<String> = OnceLock::new();
static SENDER: OnceLock<Arc<dyn SlackSender>> = OnceLock::new();

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

/// A background loop started, with its interval: its liveness gauge and
/// panic counter exist from now on, and the watchdog expects a tick from it
/// every interval.
pub fn start_loop(name: &'static str, interval: Duration) {
    crate::metrics::register_loop(name);
    register_loop(name, interval);
}

/// A background loop ended on purpose: the watchdog stops expecting it, so
/// its silence is not a stall.
pub fn stop_loop(name: &'static str) {
    if let Ok(mut loops) = loops().lock() {
        loops.remove(name);
    }
}

/// Run one tick of a started loop. A panic is counted, logged at ERROR and
/// alerted, and `None` comes back so the loop runs its next tick; either way
/// the tick counts toward the loop's liveness.
pub async fn guarded_tick<F: Future>(name: &'static str, tick: F) -> Option<F::Output> {
    use futures::FutureExt;
    let out = match std::panic::AssertUnwindSafe(tick).catch_unwind().await {
        Ok(out) => Some(out),
        Err(panic) => {
            crate::metrics::record_loop_panic(name);
            let message = panic_message(panic.as_ref()).to_string();
            tracing::error!(
                background_loop = name,
                panic = %message,
                "background loop tick panicked; the next tick runs"
            );
            alert(
                &format!("panic:{name}"),
                &format!(
                    ":rotating_light: Maidan's {name} loop tick panicked: {message}. The loop runs on, but the rest of that tick was skipped; the log has the details."
                ),
            )
            .await;
            None
        }
    };
    crate::metrics::record_loop_tick(name);
    note_tick(name);
    out
}

/// The message a panic carried, when it carried text.
pub(crate) fn panic_message(panic: &(dyn std::any::Any + Send)) -> &str {
    panic
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("a panic without a message")
}

fn register_loop(name: &'static str, interval: Duration) {
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

/// The started loops that are stalled at `at`, with how long each has been
/// quiet, in seconds.
fn stalled_loops(at: i64) -> Vec<(&'static str, i64)> {
    loops()
        .lock()
        .map(|loops| {
            loops
                .iter()
                .filter(|(_, (last, interval))| stalled(at, *last, *interval))
                .map(|(name, (last, _))| (*name, at - last))
                .collect()
        })
        .unwrap_or_default()
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
pub async fn alert(key: &str, text: &str) {
    let (Some(channel), Some(sender)) = (CHANNEL.get(), SENDER.get()) else {
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
    let Some(sender) = state.slack_sender.clone() else {
        return;
    };
    if SENDER.set(sender).is_err() || CHANNEL.set(channel).is_err() {
        return;
    }
    alert(
        "boot",
        &format!(
            ":white_check_mark: Maidan {} started; operator alerts reach this channel.",
            env!("CARGO_PKG_VERSION")
        ),
    )
    .await;
    loop {
        tokio::time::sleep(Duration::from_secs(WATCHDOG_SECS)).await;
        for (name, quiet) in stalled_loops(now()) {
            alert(
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
    fn a_panic_message_is_read_from_either_string_form() {
        let fixed: Box<dyn std::any::Any + Send> = Box::new("boom");
        let owned: Box<dyn std::any::Any + Send> = Box::new(String::from("bang"));
        let other: Box<dyn std::any::Any + Send> = Box::new(7_u8);
        assert_eq!(panic_message(fixed.as_ref()), "boom");
        assert_eq!(panic_message(owned.as_ref()), "bang");
        assert_eq!(panic_message(other.as_ref()), "a panic without a message");
    }

    #[derive(Default)]
    struct Recording(Mutex<Vec<(String, String)>>);

    #[async_trait::async_trait]
    impl SlackSender for Recording {
        async fn post_message(
            &self,
            channel: &str,
            text: &str,
            _thread_ts: Option<&str>,
        ) -> Result<Option<maidan_types::ExternalRef>, crate::slack::SlackError> {
            if let Ok(mut posts) = self.0.lock() {
                posts.push((channel.to_string(), text.to_string()));
            }
            Ok(None)
        }

        async fn update_message(
            &self,
            _channel: &str,
            _ts: &str,
            _text: &str,
        ) -> Result<(), crate::slack::SlackError> {
            Ok(())
        }
    }

    /// A panicking tick comes back as `None`, so its loop runs on, and the
    /// operator channel is told, naming the loop and the panic.
    #[tokio::test]
    async fn a_panicking_tick_is_contained_and_alerts_the_channel() {
        let recording = Arc::new(Recording::default());
        let _ = SENDER.set(recording.clone());
        let _ = CHANNEL.set("C0123ABCDE".into());
        start_loop("test_panicking", Duration::from_secs(60));
        let out: Option<()> = guarded_tick("test_panicking", async { panic!("boom") }).await;
        assert_eq!(out, None);
        assert_eq!(guarded_tick("test_panicking", async { 7 }).await, Some(7));
        let posts = recording.0.lock().unwrap().clone();
        assert!(
            posts.iter().any(|(channel, text)| channel == "C0123ABCDE"
                && text.contains("test_panicking loop tick panicked: boom")),
            "{posts:?}"
        );
    }

    /// A started loop that stops ticking is reported stalled; one that
    /// stopped on purpose is not.
    #[test]
    fn a_quiet_loop_is_stalled_and_a_stopped_one_is_not() {
        start_loop("test_quiet", Duration::from_secs(60));
        start_loop("test_stopped", Duration::from_secs(60));
        stop_loop("test_stopped");
        let later = now() + 181;
        let stalled = stalled_loops(later);
        assert!(
            stalled.iter().any(|(name, _)| *name == "test_quiet"),
            "{stalled:?}"
        );
        assert!(!stalled.iter().any(|(name, _)| *name == "test_stopped"));
        note_tick("test_quiet");
        assert!(!stalled_loops(now() + 179)
            .iter()
            .any(|(name, _)| *name == "test_quiet"));
    }

    /// Every loop `main.rs` spawns starts and guards itself here, so a new
    /// background loop cannot ship without liveness, panic containment and
    /// alerts.
    #[test]
    fn every_spawned_background_loop_is_started_and_guarded() {
        let modules: [(&str, &str); 11] = [
            ("change_sweep", include_str!("change_sweep.rs")),
            ("egress_worker", include_str!("egress_worker.rs")),
            ("web_push_worker", include_str!("web_push_worker.rs")),
            ("mail_worker", include_str!("mail_worker.rs")),
            ("retention", include_str!("retention.rs")),
            ("chain_verify", include_str!("chain_verify.rs")),
            ("scheduler", include_str!("scheduler.rs")),
            ("claim_reaper", include_str!("claim_reaper.rs")),
            ("wait_sweeper", include_str!("wait_sweeper.rs")),
            ("digest", include_str!("digest.rs")),
            ("embed_repair", include_str!("embed_repair.rs")),
        ];
        for (name, source) in modules {
            assert!(
                source.contains(&format!("start_loop(\"{name}\"")),
                "{name} does not start_loop"
            );
            assert!(
                source.contains(&format!("guarded_tick(\"{name}\"")),
                "{name} does not guard its tick"
            );
        }
        let main = include_str!("main.rs");
        let spawned: Vec<&str> = main
            .match_indices("maidan_server::")
            .filter_map(|(at, _)| {
                let rest = &main[at + "maidan_server::".len()..];
                let module = rest.split("::").next()?;
                rest[module.len()..].starts_with("::run(").then_some(module)
            })
            .filter(|module| *module != "ops_alert")
            .collect();
        for module in &spawned {
            assert!(
                modules.iter().any(|(name, _)| name == module),
                "main.rs spawns {module}::run, which is not started and guarded through ops_alert"
            );
        }
        assert!(spawned.len() >= modules.len(), "{spawned:?}");
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

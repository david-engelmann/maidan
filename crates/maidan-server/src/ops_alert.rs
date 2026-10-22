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
//! Alive is not healthy. A loop whose every tick logs a warning or an error
//! (a database that refuses its queries, say) still finishes its ticks, so
//! [`TickWarnings`] counts the warnings each tick logs and the watchdog posts
//! when a loop has failed three ticks in a row: on 2026-10-21 a locked table
//! cancelled the change sweep's query on every tick for 20 minutes and the
//! stall alert never came. A post Slack refuses is retried each watchdog
//! pass rather than lost, and a watchdog that wakes long after it slept takes
//! the host to have been suspended, not every loop to have stalled at once.
//!
//! A tick that hangs is a stall once it has run [`STALL_INTERVALS`] intervals
//! or [`MAX_TICK_SECS`], whichever is sooner, so a loop that ticks hourly is
//! not given three hours. A loop that ends without saying so (it returned,
//! or a panic outside its tick unwound it) is reported on the watchdog's
//! next pass: its [`LoopGuard`] marks it as it drops.
//!
//! `MAIDAN_OPS_FAULT=<loop>:<panic|stall|fail|exit>` injects one fault into
//! one loop after boot, to prove each alert end to end on a running stack.
//! It is refused with `MAIDAN_ENV=production`, and the boot line says when
//! one is armed.
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

use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::Layer;

use crate::slack::SlackSender;
use crate::state::AppState;

/// The same alert, at most once in this many seconds.
pub const REPEAT_SECS: i64 = 900;

/// How often the watchdog looks.
const WATCHDOG_SECS: u64 = 60;

/// A loop is stalled after this many of its intervals without a tick, and
/// failing after this many ticks in a row that logged a warning.
const STALL_INTERVALS: i64 = 3;

/// A watchdog sleep that overran by this many of its own intervals means the
/// host was suspended: every loop missed its ticks together, through no
/// fault of its own.
const SUSPEND_INTERVALS: i64 = 3;

/// Posts Slack refused, waiting for the next watchdog pass. Bounded, so an
/// outage that lasts days does not grow it without end.
const MAX_PENDING: usize = 32;

/// The longest a tick may run before it is a stall, whatever its interval.
const MAX_TICK_SECS: i64 = 900;

/// Ticks a `fail` fault fails: enough for the failing alert.
const FAULT_FAIL_TICKS: u32 = 3;

/// Log targets whose warnings say a tick was slow, not that it failed.
const NOT_FAILURES: [&str; 1] = ["sqlx"];

static CHANNEL: OnceLock<String> = OnceLock::new();
static SENDER: OnceLock<Arc<dyn SlackSender>> = OnceLock::new();

/// What the watchdog knows about one started loop.
#[derive(Debug, Clone, Default)]
struct LoopState {
    /// When it last finished a tick (unix seconds).
    last_tick: i64,
    /// Its interval, in seconds.
    interval: i64,
    /// Warnings the running tick has logged so far.
    tick_warnings: u32,
    /// Ticks in a row that logged a warning.
    failing_ticks: u32,
    /// The latest of those warnings.
    last_warning: Option<String>,
    /// When the running tick started, if one is running.
    tick_started: Option<i64>,
    /// It ended without [`stop_loop`].
    exited: bool,
}

/// One injected fault, from `MAIDAN_OPS_FAULT`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultKind {
    /// The tick panics.
    Panic,
    /// The tick hangs past the stall threshold.
    Stall,
    /// Three ticks in a row log an error.
    Fail,
    /// The loop dies: a panic outside its tick unwinds it.
    Exit,
}

/// A fault armed for one loop, and how many more ticks it takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fault {
    pub loop_name: String,
    pub kind: FaultKind,
}

static FAULT: OnceLock<Mutex<Option<(Fault, u32)>>> = OnceLock::new();

static LOOPS: OnceLock<Mutex<HashMap<&'static str, LoopState>>> = OnceLock::new();

/// When each alert key last posted (unix seconds).
static SENT: OnceLock<Mutex<HashMap<String, i64>>> = OnceLock::new();

/// Alerts Slack refused, as (key, text), oldest first.
static PENDING: OnceLock<Mutex<Vec<(String, String)>>> = OnceLock::new();

fn loops() -> &'static Mutex<HashMap<&'static str, LoopState>> {
    LOOPS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn pending() -> &'static Mutex<Vec<(String, String)>> {
    PENDING.get_or_init(|| Mutex::new(Vec::new()))
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
/// every interval. Hold the guard for the loop's life: when it drops without
/// [`stop_loop`], the loop has died and the watchdog says so.
pub fn start_loop(name: &'static str, interval: Duration) -> LoopGuard {
    crate::metrics::register_loop(name);
    register_loop(name, interval);
    LoopGuard(name)
}

/// Marks its loop exited when it drops, which is when the loop's task ends:
/// by returning, by a panic outside a guarded tick, or by being aborted.
#[derive(Debug)]
#[must_use = "bind it for the loop's life; dropped at once, the loop reads as exited"]
pub struct LoopGuard(&'static str);

impl Drop for LoopGuard {
    fn drop(&mut self) {
        if let Ok(mut loops) = loops().lock() {
            if let Some(state) = loops.get_mut(self.0) {
                state.exited = true;
            }
        }
    }
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
    let fault = due_fault(name);
    if fault == Some(FaultKind::Exit) {
        // Outside the catch below, so it unwinds the loop's task the way a
        // real bug in the loop body would.
        panic!("MAIDAN_OPS_FAULT: {name} loop exits");
    }
    let hang = Duration::from_secs(u64::try_from(tick_limit(interval_of(name)) + 120).unwrap_or(0));
    let tick = async move {
        match fault {
            Some(FaultKind::Panic) => panic!("MAIDAN_OPS_FAULT: {name} tick panics"),
            Some(FaultKind::Stall) => tokio::time::sleep(hang).await,
            Some(FaultKind::Fail) => tracing::error!("MAIDAN_OPS_FAULT: {name} tick fails"),
            Some(FaultKind::Exit) | None => {}
        }
        tick.await
    };
    begin_tick(name);
    let out = match std::panic::AssertUnwindSafe(TICK_LOOP.scope(name, tick))
        .catch_unwind()
        .await
    {
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
        loops.insert(
            name,
            LoopState {
                last_tick: now(),
                interval,
                ..LoopState::default()
            },
        );
    }
}

fn begin_tick(name: &str) {
    if let Ok(mut loops) = loops().lock() {
        if let Some(state) = loops.get_mut(name) {
            state.tick_warnings = 0;
            state.tick_started = Some(now());
        }
    }
}

fn interval_of(name: &str) -> i64 {
    loops()
        .lock()
        .ok()
        .and_then(|loops| loops.get(name).map(|state| state.interval))
        .unwrap_or(0)
}

/// How long one tick of a loop with this interval may run.
fn tick_limit(interval: i64) -> i64 {
    interval.saturating_mul(STALL_INTERVALS).min(MAX_TICK_SECS)
}

/// Read `MAIDAN_OPS_FAULT`. Unset or blank is none; a malformed value, or
/// any value with `MAIDAN_ENV=production`, is an error boot refuses.
pub fn fault_from_env() -> Result<Option<Fault>, String> {
    parse_fault(
        std::env::var("MAIDAN_OPS_FAULT").ok(),
        crate::config::is_production(),
    )
}

fn parse_fault(raw: Option<String>, production: bool) -> Result<Option<Fault>, String> {
    let Some(raw) = raw
        .map(|raw| raw.trim().to_string())
        .filter(|raw| !raw.is_empty())
    else {
        return Ok(None);
    };
    if production {
        return Err("MAIDAN_OPS_FAULT cannot be set when MAIDAN_ENV=production".into());
    }
    let (loop_name, kind) = raw.split_once(':').ok_or_else(|| {
        format!("MAIDAN_OPS_FAULT must be <loop>:<panic|stall|fail|exit>, not {raw:?}")
    })?;
    let kind = match kind {
        "panic" => FaultKind::Panic,
        "stall" => FaultKind::Stall,
        "fail" => FaultKind::Fail,
        "exit" => FaultKind::Exit,
        other => {
            return Err(format!(
                "MAIDAN_OPS_FAULT fault must be panic, stall, fail or exit, not {other:?}"
            ))
        }
    };
    if loop_name.is_empty() {
        return Err("MAIDAN_OPS_FAULT names no loop".into());
    }
    Ok(Some(Fault {
        loop_name: loop_name.to_string(),
        kind,
    }))
}

/// Arm a fault: the named loop's next ticks take it.
pub fn arm_fault(fault: Fault) {
    let ticks = if fault.kind == FaultKind::Fail {
        FAULT_FAIL_TICKS
    } else {
        1
    };
    tracing::warn!(loop_name = fault.loop_name, kind = ?fault.kind, "MAIDAN_OPS_FAULT armed");
    if let Ok(mut armed) = FAULT.get_or_init(|| Mutex::new(None)).lock() {
        *armed = Some((fault, ticks));
    }
}

/// The fault this tick of `name` takes, if one is armed for it.
fn due_fault(name: &str) -> Option<FaultKind> {
    let mut armed = FAULT.get()?.lock().ok()?;
    let (fault, left) = armed.as_mut()?;
    if fault.loop_name != name || *left == 0 {
        return None;
    }
    *left -= 1;
    Some(fault.kind)
}

/// The armed fault, for the boot line.
fn armed_fault() -> Option<Fault> {
    FAULT
        .get()?
        .lock()
        .ok()?
        .as_ref()
        .map(|(fault, _)| fault.clone())
}

fn note_warning(name: &str, warning: String) {
    if let Ok(mut loops) = loops().lock() {
        if let Some(state) = loops.get_mut(name) {
            state.tick_warnings = state.tick_warnings.saturating_add(1);
            state.last_warning = Some(warning);
        }
    }
}

/// A registered loop finished a tick.
pub fn note_tick(name: &'static str) {
    if let Ok(mut loops) = loops().lock() {
        if let Some(state) = loops.get_mut(name) {
            state.last_tick = now();
            state.tick_started = None;
            if state.tick_warnings > 0 {
                state.failing_ticks = state.failing_ticks.saturating_add(1);
            } else {
                state.failing_ticks = 0;
                state.last_warning = None;
            }
        }
    }
}

/// Counts the warnings and errors logged inside a guarded tick against its
/// loop, so a loop that fails every tick is not mistaken for a healthy one.
/// Installed with the server's other log layers.
#[derive(Debug, Default, Clone, Copy)]
pub struct TickWarnings;

tokio::task_local! {
    /// The loop whose tick is running on this task. A task-local rather than
    /// a span: a span per tick would prefix every log line and ship one more
    /// trace span per tick to the OTLP collector.
    static TICK_LOOP: &'static str;
}

/// An event's message and error, as text.
#[derive(Default)]
struct Fields {
    message: Option<String>,
    error: Option<String>,
}

impl Visit for Fields {
    fn record_str(&mut self, field: &Field, value: &str) {
        match field.name() {
            "message" => self.message = Some(value.to_string()),
            "error" => self.error = Some(value.to_string()),
            _ => {}
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        match field.name() {
            "message" => self.message = Some(format!("{value:?}")),
            "error" => self.error = Some(format!("{value:?}")),
            _ => {}
        }
    }
}

impl<S: Subscriber> Layer<S> for TickWarnings {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let meta = event.metadata();
        if *meta.level() > Level::WARN
            || NOT_FAILURES
                .iter()
                .any(|target| meta.target().starts_with(target))
        {
            return;
        }
        let Ok(name) = TICK_LOOP.try_with(|name| *name) else {
            return;
        };
        let mut fields = Fields::default();
        event.record(&mut fields);
        let warning = match (fields.message, fields.error) {
            (Some(message), Some(error)) => format!("{message}: {error}"),
            (Some(text), None) | (None, Some(text)) => text,
            (None, None) => meta.name().to_string(),
        };
        note_warning(name, warning);
    }
}

fn now() -> i64 {
    chrono::Utc::now().timestamp()
}

/// At most `max` characters of `text`, so one long error cannot fill a post.
fn truncate(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((end, _)) => format!("{}…", &text[..end]),
        None => text.to_string(),
    }
}

/// Whether a loop that last ticked at `last`, every `interval` seconds, is
/// stalled at `at`.
fn stalled(at: i64, last: i64, interval: i64) -> bool {
    at - last > interval.saturating_mul(STALL_INTERVALS)
}

/// Whether a loop is stalled at `at`: quiet for three intervals, or in one
/// tick for longer than [`tick_limit`].
fn loop_stalled(at: i64, state: &LoopState) -> bool {
    stalled(at, state.last_tick, state.interval)
        || state
            .tick_started
            .is_some_and(|started| at - started > tick_limit(state.interval))
}

/// The started loops that are stalled at `at`, with how long each has been
/// quiet, in seconds.
fn stalled_loops(at: i64) -> Vec<(&'static str, i64)> {
    loops()
        .lock()
        .map(|loops| {
            loops
                .iter()
                .filter(|(_, state)| !state.exited && loop_stalled(at, state))
                .map(|(name, state)| (*name, at - state.last_tick))
                .collect()
        })
        .unwrap_or_default()
}

/// The started loops whose last ticks all logged a warning, with how many
/// in a row and the latest warning.
fn failing_loops() -> Vec<(&'static str, u32, String)> {
    loops()
        .lock()
        .map(|loops| {
            loops
                .iter()
                .filter(|(_, state)| i64::from(state.failing_ticks) >= STALL_INTERVALS)
                .map(|(name, state)| {
                    (
                        *name,
                        state.failing_ticks,
                        state.last_warning.clone().unwrap_or_default(),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The loops that ended without [`stop_loop`], taken out of the watch: each
/// is reported once.
fn take_exited_loops() -> Vec<&'static str> {
    loops()
        .lock()
        .map(|mut loops| {
            let exited: Vec<&'static str> = loops
                .iter()
                .filter(|(_, state)| state.exited)
                .map(|(name, _)| *name)
                .collect();
            for name in &exited {
                loops.remove(name);
            }
            exited
        })
        .unwrap_or_default()
}

/// The loop an armed fault names when no loop of that name has started.
/// A misspelt name would otherwise arm nothing, and the drill would wait
/// for an alert that can never come.
fn unstarted_fault_loop(
    fault: Option<&Fault>,
    started: &HashMap<&'static str, LoopState>,
) -> Option<String> {
    let fault = fault?;
    (!started.contains_key(fault.loop_name.as_str())).then(|| fault.loop_name.clone())
}

/// Whether a watchdog sleep of `secs` seconds that took `slept` means the
/// host was suspended.
fn suspended(slept: i64, secs: i64) -> bool {
    slept > secs.saturating_mul(SUSPEND_INTERVALS)
}

/// After a suspend every loop starts its clock again: the ticks it missed
/// were the host's, and it gets its full intervals to tick again.
fn restart_clocks(at: i64) {
    if let Ok(mut loops) = loops().lock() {
        for state in loops.values_mut() {
            state.last_tick = at;
            if state.tick_started.is_some() {
                state.tick_started = Some(at);
            }
        }
    }
}

/// Keep a refused post for the next pass: a newer text for the same key
/// replaces the older, and past [`MAX_PENDING`] the oldest goes.
fn queue_retry(queue: &mut Vec<(String, String)>, key: &str, text: &str) {
    queue.retain(|(queued, _)| queued != key);
    queue.push((key.to_string(), text.to_string()));
    if queue.len() > MAX_PENDING {
        let excess = queue.len() - MAX_PENDING;
        queue.drain(..excess);
    }
}

/// Post every alert Slack refused before; one refused again waits again.
async fn retry_pending() {
    let (Some(channel), Some(sender)) = (CHANNEL.get(), SENDER.get()) else {
        return;
    };
    let queued = pending()
        .lock()
        .map(|mut queue| std::mem::take(&mut *queue))
        .unwrap_or_default();
    for (key, text) in queued {
        if let Err(err) = sender.post_message(channel, &text, None).await {
            tracing::warn!(error = %err, key, "operator alert not posted; retrying next pass");
            if let Ok(mut queue) = pending().lock() {
                queue_retry(&mut queue, &key, &text);
            }
        }
    }
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
/// last [`REPEAT_SECS`]. A no-op when alerts are off. A post Slack refuses
/// is logged at WARN and retried on each watchdog pass until it goes.
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
        tracing::warn!(error = %err, key, "operator alert not posted; retrying next pass");
        if let Ok(mut queue) = pending().lock() {
            queue_retry(&mut queue, key, text);
        }
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
    let armed = armed_fault()
        .map(|fault| {
            format!(
                " Fault injection is armed: {} {:?} (MAIDAN_OPS_FAULT).",
                fault.loop_name, fault.kind
            )
        })
        .unwrap_or_default();
    alert(
        "boot",
        &format!(
            ":white_check_mark: Maidan {} started; operator alerts reach this channel.{armed}",
            env!("CARGO_PKG_VERSION")
        ),
    )
    .await;
    let watchdog_secs = i64::try_from(WATCHDOG_SECS).unwrap_or(i64::MAX);
    // Every loop registers at boot, so one watchdog pass is long enough to
    // tell a fault aimed at a loop that never started.
    let mut first_pass = true;
    loop {
        let slept_from = now();
        tokio::time::sleep(Duration::from_secs(WATCHDOG_SECS)).await;
        let at = now();
        if std::mem::take(&mut first_pass) {
            let unstarted = loops()
                .lock()
                .ok()
                .and_then(|started| unstarted_fault_loop(armed_fault().as_ref(), &started));
            if let Some(name) = unstarted {
                alert(
                    &format!("fault-unstarted:{name}"),
                    &format!(
                        ":warning: MAIDAN_OPS_FAULT is armed for {name}, which never started; no fault will fire. Check the loop name."
                    ),
                )
                .await;
            }
        }
        if suspended(at - slept_from, watchdog_secs) {
            tracing::info!(
                away_secs = at - slept_from,
                "the host was suspended; background loop clocks restart"
            );
            restart_clocks(at);
        }
        retry_pending().await;
        for name in take_exited_loops() {
            alert(
                &format!("exited:{name}"),
                &format!(
                    ":rotating_light: Maidan's {name} loop has exited. The work it drives has stopped until the server restarts; the log has why."
                ),
            )
            .await;
        }
        for (name, ticks, warning) in failing_loops() {
            alert(
                &format!("failing:{name}"),
                &format!(
                    ":rotating_light: Maidan's {name} loop is running but its last {ticks} ticks failed. Latest: {}",
                    truncate(&warning, 300)
                ),
            )
            .await;
        }
        for (name, quiet) in stalled_loops(at) {
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
    fn a_fault_for_a_loop_that_never_started_is_named_and_a_started_one_is_not() {
        let mut started = HashMap::new();
        started.insert("outbox", LoopState::default());
        let fault = |name: &str| Fault {
            loop_name: name.to_string(),
            kind: FaultKind::Stall,
        };
        assert_eq!(
            unstarted_fault_loop(Some(&fault("outbx")), &started).as_deref(),
            Some("outbx")
        );
        assert_eq!(unstarted_fault_loop(Some(&fault("outbox")), &started), None);
        assert_eq!(unstarted_fault_loop(None, &started), None);
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
        let _test_panicking = start_loop("test_panicking", Duration::from_secs(60));
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

    /// A loop that finishes every tick but logs a warning in each is failing
    /// after three in a row, named with its latest warning; one clean tick
    /// clears it. The 2026-10-21 lock test: the sweep's query was cancelled
    /// on every tick and the loop still looked alive.
    #[tokio::test]
    async fn a_loop_whose_every_tick_warns_is_failing_and_a_clean_tick_clears_it() {
        use tracing_subscriber::layer::SubscriberExt;
        let _log =
            tracing::subscriber::set_default(tracing_subscriber::registry().with(TickWarnings));
        let is_failing = || {
            failing_loops()
                .into_iter()
                .find(|(name, _, _)| *name == "test_failing")
        };
        let _test_failing = start_loop("test_failing", Duration::from_secs(60));
        for _ in 0..2 {
            guarded_tick("test_failing", async {
                tracing::warn!(
                    error = "canceling statement due to lock timeout",
                    "workspaces not read"
                );
            })
            .await;
        }
        assert!(
            is_failing().is_none(),
            "two failed ticks are not yet failing"
        );
        guarded_tick("test_failing", async {
            tracing::warn!(
                error = "canceling statement due to lock timeout",
                "workspaces not read"
            );
        })
        .await;
        let (_, ticks, warning) = is_failing().expect("failing after three");
        assert_eq!(ticks, 3);
        assert_eq!(
            warning,
            "workspaces not read: canceling statement due to lock timeout"
        );
        guarded_tick("test_failing", async {}).await;
        assert!(is_failing().is_none(), "a clean tick clears it");
    }

    /// A slow statement is not a failed tick, and a warning logged outside
    /// any tick counts against no loop.
    #[tokio::test]
    async fn a_slow_statement_or_an_untracked_warning_is_not_a_failure() {
        use tracing_subscriber::layer::SubscriberExt;
        let _log =
            tracing::subscriber::set_default(tracing_subscriber::registry().with(TickWarnings));
        let _test_slow = start_loop("test_slow", Duration::from_secs(60));
        for _ in 0..3 {
            guarded_tick("test_slow", async {
                tracing::warn!(target: "sqlx::query", "slow statement: execution time exceeded alert threshold");
            })
            .await;
            tracing::warn!("a warning from no loop");
            note_tick("test_slow");
        }
        assert!(!failing_loops()
            .iter()
            .any(|(name, _, _)| *name == "test_slow"));
    }

    /// A post Slack refused is kept for the next pass, once per key with its
    /// newest text, and the queue keeps only the newest [`MAX_PENDING`].
    #[test]
    fn a_refused_post_waits_for_the_next_pass() {
        let mut queue = Vec::new();
        queue_retry(&mut queue, "stall:a", "first");
        queue_retry(&mut queue, "panic:b", "boom");
        queue_retry(&mut queue, "stall:a", "second");
        assert_eq!(
            queue,
            vec![
                ("panic:b".to_string(), "boom".to_string()),
                ("stall:a".to_string(), "second".to_string()),
            ]
        );
        for n in 0..MAX_PENDING + 5 {
            queue_retry(&mut queue, &format!("k{n}"), "t");
        }
        assert_eq!(queue.len(), MAX_PENDING);
        assert_eq!(queue.first().map(|(key, _)| key.as_str()), Some("k5"));
    }

    /// A watchdog that slept far past its interval saw a suspended host, and
    /// every loop gets its full intervals again rather than an alert.
    #[test]
    fn a_suspended_host_restarts_the_clocks_instead_of_alerting() {
        assert!(!suspended(180, 60));
        assert!(suspended(181, 60));
        let _test_suspended = start_loop("test_suspended", Duration::from_secs(60));
        let woke = now() + 3_600;
        assert!(stalled_loops(woke)
            .iter()
            .any(|(name, _)| *name == "test_suspended"));
        restart_clocks(woke);
        assert!(!stalled_loops(woke + 60)
            .iter()
            .any(|(name, _)| *name == "test_suspended"));
    }

    /// A tick that hangs is a stall after three intervals or 15 minutes,
    /// whichever is sooner, so an hourly loop is not given three hours; an
    /// idle long-interval loop between ticks is not a stall.
    #[test]
    fn a_hung_tick_is_a_stall_before_three_long_intervals() {
        let at = 100_000;
        let hourly = |started: Option<i64>| LoopState {
            last_tick: at - 1_000,
            interval: 3_600,
            tick_started: started,
            ..LoopState::default()
        };
        assert!(!loop_stalled(at, &hourly(None)));
        assert!(!loop_stalled(at, &hourly(Some(at - MAX_TICK_SECS))));
        assert!(loop_stalled(at, &hourly(Some(at - MAX_TICK_SECS - 1))));
        let quick = LoopState {
            last_tick: at - 10,
            interval: 60,
            tick_started: Some(at - 181),
            ..LoopState::default()
        };
        assert!(loop_stalled(at, &quick));
    }

    /// A loop whose guard drops without `stop_loop` is reported exited once
    /// and is no longer watched; one stopped on purpose is not reported.
    #[test]
    fn a_loop_that_ends_unannounced_is_reported_once() {
        drop(start_loop("test_exits", Duration::from_secs(60)));
        let stopped = start_loop("test_ends_on_purpose", Duration::from_secs(60));
        stop_loop("test_ends_on_purpose");
        drop(stopped);
        let exited = take_exited_loops();
        assert!(exited.contains(&"test_exits"), "{exited:?}");
        assert!(!exited.contains(&"test_ends_on_purpose"));
        assert!(!take_exited_loops().contains(&"test_exits"));
        assert!(!stalled_loops(now() + 10_000)
            .iter()
            .any(|(name, _)| *name == "test_exits"));
    }

    #[test]
    fn a_fault_names_one_loop_and_one_fault_and_never_runs_in_production() {
        assert_eq!(parse_fault(None, false), Ok(None));
        assert_eq!(parse_fault(Some("  ".into()), true), Ok(None));
        assert_eq!(
            parse_fault(Some("change_sweep:fail".into()), false),
            Ok(Some(Fault {
                loop_name: "change_sweep".into(),
                kind: FaultKind::Fail,
            }))
        );
        for bad in ["change_sweep", "change_sweep:explode", ":panic"] {
            assert!(parse_fault(Some(bad.into()), false).is_err(), "{bad}");
        }
        let refused = parse_fault(Some("change_sweep:panic".into()), true);
        assert!(refused.is_err_and(|e| e.contains("MAIDAN_ENV=production")));
    }

    /// Each armed fault reaches only the loop it names: `fail` fails three
    /// ticks, `panic` is contained like any panic, and `exit` unwinds the
    /// loop's task, which is then reported exited.
    #[tokio::test]
    async fn an_armed_fault_takes_only_the_named_loop() {
        use tracing_subscriber::layer::SubscriberExt;
        let _log =
            tracing::subscriber::set_default(tracing_subscriber::registry().with(TickWarnings));
        let _fault_loop = start_loop("test_fault", Duration::from_secs(60));
        arm_fault(Fault {
            loop_name: "test_fault".into(),
            kind: FaultKind::Fail,
        });
        assert_eq!(due_fault("test_other"), None);
        for _ in 0..FAULT_FAIL_TICKS {
            guarded_tick("test_fault", async {}).await;
        }
        assert!(failing_loops()
            .iter()
            .any(|(name, _, _)| *name == "test_fault"));
        guarded_tick("test_fault", async {}).await;
        assert!(!failing_loops()
            .iter()
            .any(|(name, _, _)| *name == "test_fault"));

        arm_fault(Fault {
            loop_name: "test_fault".into(),
            kind: FaultKind::Panic,
        });
        assert_eq!(guarded_tick("test_fault", async { 1 }).await, None);
        assert_eq!(guarded_tick("test_fault", async { 2 }).await, Some(2));

        arm_fault(Fault {
            loop_name: "test_fault_exit".into(),
            kind: FaultKind::Exit,
        });
        let task = tokio::spawn(async {
            let _watched = start_loop("test_fault_exit", Duration::from_secs(60));
            loop {
                guarded_tick("test_fault_exit", async {}).await;
            }
        });
        assert!(task.await.is_err_and(|e| e.is_panic()));
        assert!(take_exited_loops().contains(&"test_fault_exit"));
    }

    #[test]
    fn a_long_warning_is_cut_short() {
        assert_eq!(truncate("abc", 5), "abc");
        assert_eq!(truncate("abcdef", 3), "abc…");
    }

    /// A started loop that stops ticking is reported stalled; one that
    /// stopped on purpose is not.
    #[test]
    fn a_quiet_loop_is_stalled_and_a_stopped_one_is_not() {
        let _test_quiet = start_loop("test_quiet", Duration::from_secs(60));
        let _test_stopped = start_loop("test_stopped", Duration::from_secs(60));
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
                source.contains(&format!(
                    "let _watched = crate::ops_alert::start_loop(\"{name}\""
                )),
                "{name} does not start_loop and hold its guard for the loop's life"
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

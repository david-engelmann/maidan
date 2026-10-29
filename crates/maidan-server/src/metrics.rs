//! Prometheus metrics (Track T.4).

use std::{
    sync::{atomic::Ordering, Mutex, Once, OnceLock, PoisonError},
    time::Instant,
};

use maidan_bus::HydrateSnapshot;

use axum::{
    extract::Request,
    extract::State,
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use chrono::Utc;
#[cfg(feature = "otel")]
use maidan_observability::{
    build_otlp_metrics_recorder, otlp_metrics_endpoint_from_env, otlp_metrics_interval_from_env,
    MeterGuard, MetricsPushConfig,
};
use metrics::{counter, describe_counter, describe_gauge, describe_histogram, gauge, histogram};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
#[cfg(feature = "otel")]
use metrics_util::layers::FanoutBuilder;

use crate::state::AppState;

static PROMETHEUS: OnceLock<PrometheusHandle> = OnceLock::new();
#[cfg(feature = "otel")]
static OTLP_METER: OnceLock<MeterGuard> = OnceLock::new();
static INIT: Once = Once::new();
static LAST_HYDRATE: Mutex<Option<HydrateSnapshot>> = Mutex::new(None);
static LAST_READ_ROUTING: Mutex<(u64, u64)> = Mutex::new((0, 0));
static LAST_SEARCH_READ_ROUTING: Mutex<(u64, u64)> = Mutex::new((0, 0));

fn spawn_prometheus_upkeep(handle: PrometheusHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_secs(5));
        handle.run_upkeep();
    });
}

/// Install the global metrics recorder, fanning out to OTLP push when
/// configured.
#[cfg(feature = "otel")]
fn install_global_recorder(prom_recorder: metrics_exporter_prometheus::PrometheusRecorder) {
    if let Some(endpoint) = otlp_metrics_endpoint_from_env() {
        let service_name =
            std::env::var("OTLP_SERVICE_NAME").unwrap_or_else(|_| "maidan-server".to_string());
        let push_config = MetricsPushConfig {
            service_name,
            endpoint,
            interval: otlp_metrics_interval_from_env(),
        };
        match build_otlp_metrics_recorder(&push_config) {
            Ok((meter_guard, otel_recorder)) => {
                let fanout = FanoutBuilder::default()
                    .add_recorder(prom_recorder)
                    .add_recorder(otel_recorder)
                    .build();
                if let Err(err) = metrics::set_global_recorder(fanout) {
                    tracing::error!(%err, "failed to install metrics recorder");
                }
                let _ = OTLP_METER.set(meter_guard);
                tracing::info!(
                    endpoint = %push_config.endpoint,
                    interval_secs = push_config.interval.as_secs(),
                    "OTLP metrics push enabled (Prometheus scrape unchanged)"
                );
            }
            Err(err) => {
                tracing::error!(%err, "OTLP metrics recorder init failed; Prometheus only");
                if let Err(err) = metrics::set_global_recorder(prom_recorder) {
                    tracing::error!(%err, "failed to install metrics recorder");
                }
            }
        }
    } else if let Err(err) = metrics::set_global_recorder(prom_recorder) {
        tracing::error!(%err, "failed to install metrics recorder");
    }
}

/// Install the global metrics recorder (Prometheus only — the no-`otel` build).
#[cfg(not(feature = "otel"))]
fn install_global_recorder(prom_recorder: metrics_exporter_prometheus::PrometheusRecorder) {
    if let Err(err) = metrics::set_global_recorder(prom_recorder) {
        tracing::error!(%err, "failed to install metrics recorder");
    }
}

/// Install the global metrics recorder (idempotent).
pub fn init() {
    INIT.call_once(|| {
        let prom_recorder = PrometheusBuilder::new().build_recorder();
        let handle = prom_recorder.handle();
        install_global_recorder(prom_recorder);
        spawn_prometheus_upkeep(handle.clone());
        let _ = PROMETHEUS.set(handle);
        describe_counter!(
            "http.server.request_total",
            "Total HTTP requests served (excludes WebSocket upgrade)"
        );
        describe_histogram!(
            "http.server.request_duration_seconds",
            "HTTP request latency in seconds"
        );
        describe_counter!(
            "maidan_http_shed_total",
            "HTTP requests refused with 503 because the in-flight ceiling was reached"
        );
        describe_counter!(
            "maidan_artifact_upload_budget_refused_total",
            "Artifact uploads refused with 503 because the in-memory upload byte budget was full"
        );
        describe_counter!(
            "maidan_http_panics_total",
            "HTTP requests whose handler panicked, answered with a 500 problem"
        );
        describe_gauge!(
            "maidan_http_in_flight_requests",
            "HTTP requests holding an in-flight permit (0 when shedding is off)"
        );
        describe_counter!(
            "maidan_audit_write_failures_total",
            "Audit rows that failed to write; the change they record went through without it"
        );
        describe_counter!(
            "maidan_event_append_failures_total",
            "Events lost after retries although their domain row committed"
        );
        describe_counter!(
            "maidan_bus_lag_total",
            "In-process bus subscriber lag events (one per BusItem::Lagged)"
        );
        describe_histogram!(
            "maidan_bus_lag_skipped",
            "Skipped events reported when a subscriber lagged"
        );
        describe_counter!(
            "maidan_subscribe_replay_total",
            "Subscribe recovery actions after lag or failed auto-replay"
        );
        describe_counter!(
            "maidan_bus_lag_resume_total",
            "Internal bus consumers that resumed from the event log after BusItem::Lagged"
        );
        describe_gauge!(
            "maidan_content_keys_refused",
            "Content key rows the last rewrap pass left because the key custody refused them (0 when none)"
        );
        describe_gauge!(
            "maidan_workspace_keys_undestroyed",
            "Retired workspace keys whose custody key the last rewrap pass could not destroy, e.g. a KMS key disabled by hand (0 when none)"
        );
        describe_counter!(
            "maidan_custody_stray_keys_destroyed_total",
            "Custody keys no workspace key row referred to (a wrap whose transaction never committed), destroyed by the stray-key sweep"
        );
        describe_gauge!(
            "maidan_custody_stray_keys_failed",
            "Stray custody keys the last sweep could not destroy (0 when none)"
        );
        describe_gauge!(
            "maidan_indexer_last_event_age_seconds",
            "Seconds since the background indexer last made progress (0 if never); an idle instance ages here, so alert on maidan_indexer_pending_age_seconds"
        );
        describe_gauge!(
            "maidan_indexer_pending_age_seconds",
            "Seconds since the oldest indexable event the indexer has not handled was stored (0 when caught up)"
        );
        describe_gauge!(
            "maidan_indexer_events_behind",
            "Indexable events the indexer has not handled, counted up to 100000 (0 when caught up)"
        );
        describe_gauge!(
            "maidan_bus_listener_ok",
            "Postgres LISTEN listener health (1=ok, 0=degraded)"
        );
        describe_gauge!(
            "maidan_bus_listener_errors_total",
            "Cumulative Postgres LISTEN listener errors since process start"
        );
        describe_counter!(
            "maidan_bus_notify_hydrate_total",
            "Postgres NOTIFY pointer hydrations by outcome"
        );
        describe_gauge!(
            "maidan_threads_stalled",
            "Threads held on a live lease whose holder has shown no progress for MAIDAN_STALL_SECS: a claim that only renews"
        );
        describe_gauge!(
            "maidan_outbox_pending",
            "Unpublished outbox rows awaiting relay (Postgres)"
        );
        describe_counter!(
            "maidan_outbox_relay_total",
            "Outbox relay publish attempts by outcome"
        );
        describe_gauge!(
            "maidan_outbox_quarantined",
            "Outbox rows quarantined after max relay attempts (unpublished)"
        );
        describe_gauge!(
            "maidan_outbox_oldest_pending_seconds",
            "Age in seconds of the oldest relayable pending outbox row"
        );
        describe_counter!(
            "maidan_automation_delivery_total",
            "Automation HTTP deliveries (slash/fsm) by outcome"
        );
        describe_counter!(
            "maidan_egress_retry_deferred_total",
            "Outbound retries the per-host retry budget held back and rescheduled, by \
             worker (webhook/automation/egress/mail); a deferral is not a failed attempt"
        );
        describe_counter!(
            "maidan_a2a_push_total",
            "A2A task push notifications by outcome (ok/failed after retries)"
        );
        describe_counter!(
            "maidan_federation_peer_polls_total",
            "Federation peer polls by outcome (ok/refused/no_secret/failed); a failed \
             peer is skipped and the other peers are still polled"
        );
        describe_counter!(
            "maidan_event_append_failures_total",
            "Domain events that failed to append to the log after retries (a lost \
             event: the domain row committed but no event was persisted)"
        );
        describe_counter!(
            "maidan_retention_pruned_total",
            "Rows deleted by the data-retention sweeper, by table"
        );
        describe_counter!(
            "maidan_task_schedules_fired_total",
            "Task schedules fired by the scheduler sweeper (a thread was created), by outcome"
        );
        describe_counter!(
            "maidan_claims_reaped_total",
            "Lapsed claim leases the claim reaper returned to the queue (each with a ClaimExpired)"
        );
        describe_counter!(
            "maidan_claims_unacknowledged_total",
            "Leased claims reported as never acknowledged (each with a ClaimUnacknowledged)"
        );
        describe_counter!(
            "maidan_human_waits_returned_total",
            "Threads returned to the queue because their agent's wait on a person went unanswered (each with a WaitTimedOut, policy return_to_queue)"
        );
        describe_counter!(
            "maidan_wait_timed_out_total",
            "Thread wait timers fired by the wait sweeper, by escalation policy (notify/park)"
        );
        describe_counter!(
            "maidan_replica_reads_total",
            "Store reads routed to the primary vs a read replica (LSN-token read routing)"
        );
        describe_counter!(
            "maidan_search_replica_reads_total",
            "Message-search reads routed to the primary vs a read replica (LSN-token read routing)"
        );
        describe_gauge!(
            "maidan_replica_lag_bytes",
            "Read-replica lag in WAL bytes (primary write LSN minus replica replay LSN)"
        );
        describe_counter!(
            "maidan_notifications_created_total",
            "Per-recipient notifications written by the notification router, by kind"
        );
        describe_counter!(
            "maidan_notifications_suppressed_total",
            "Notifications the router did NOT write, by reason (e.g. a muted preference)"
        );
        describe_counter!(
            "maidan_email_delivered_total",
            "Notification emails the router attempted to send, by outcome (sent/failed)"
        );
        describe_counter!(
            "maidan_web_push_delivered_total",
            "Web Push notifications by outcome (sent/failed/queued/retry/dead/pruned/skipped_present/skipped_vapid_unset)"
        );
        describe_gauge!(
            "maidan_background_loop_last_tick_seconds",
            "Unix time the background loop started or last finished a tick, by loop: a stalled loop stops moving it"
        );
        describe_counter!(
            "maidan_background_loop_panics_total",
            "Background loop ticks that panicked, by loop; the loop logs the panic and runs its next tick"
        );
        describe_histogram!(
            "maidan_thread_claim_duration_seconds",
            "Seconds from a claim's acknowledgement to its stored result, by harness and kind (closed sets)"
        );
        describe_counter!(
            "maidan_thread_results_total",
            "Stored thread results by harness (codex, claude, deepseek, grok, gemini, pi, maidan, other), kind (redteam, security, review, plan, build, fix, other), status and verdict (pass, fix, merge, block, reject, other, none); every label a closed set"
        );
        describe_counter!(
            "maidan_result_deliveries_total",
            "Thread-result delivery dispositions by surface (github, github_branch, slack, linear, other) and outcome (enqueued, skipped, delivered, failed)"
        );
        describe_counter!(
            "maidan_authorization_decisions_total",
            "Content-free authorization decisions by surface, action, outcome, and resource kind"
        );
        describe_histogram!(
            "maidan_automation_delivery_duration_seconds",
            "Automation HTTP delivery attempt latency"
        );
        describe_counter!(
            "maidan_usage_tokens_total",
            "Accepted usage tokens by tier (input, output, cache_read, cache_write_5m, cache_write_1h) and provider family (anthropic, openai, google, xai, other)"
        );
        describe_counter!(
            "maidan_usage_usd_micros_total",
            "Accepted usage charges in micro-USD, by provider family (anthropic, openai, google, xai, other)"
        );
        describe_counter!(
            "maidan_usage_uncached_usd_micros_total",
            "What the same tokens would have cost with no cache, in micro-USD"
        );
        describe_counter!(
            "maidan_usage_saved_usd_micros_total",
            "Micro-USD saved against the uncached price"
        );
        describe_counter!(
            "maidan_usage_lane_saved_usd_micros_total",
            "Micro-USD saved by batch and flex calls against their standard price, as reported"
        );
        describe_counter!(
            "maidan_usage_write_premium_usd_micros_total",
            "Micro-USD paid above the uncached price"
        );
    });
}

pub fn record_automation_delivery(success: bool) {
    let outcome = if success { "success" } else { "failure" };
    counter!(
        "maidan_automation_delivery_total",
        "outcome" => outcome.to_string()
    )
    .increment(1);
}

pub fn record_automation_delivery_duration(elapsed: std::time::Duration) {
    histogram!("maidan_automation_delivery_duration_seconds").record(elapsed.as_secs_f64());
}

/// A domain event failed to append to the log after retries — the domain row
/// committed but no event was persisted, so downstream consumers (WS/MCP
/// notifications, at-least-once delivery, the indexer) will never see it. Alert
/// on any non-zero rate.
pub fn record_event_append_failure() {
    counter!("maidan_event_append_failures_total").increment(1);
}

/// Rows deleted by the retention sweeper for `table`.
pub fn record_retention_pruned(table: &str, count: u64) {
    counter!("maidan_retention_pruned_total", "table" => table.to_string()).increment(count);
}

/// One workspace's chain verified by the sweeper.
///
/// `outcome` is `ok` | `broken` | `error`. **`broken` and `error` are distinct
/// on purpose**: verified-and-broken is a tamper, could-not-verify is a
/// database problem, and an alert that cannot tell them apart will be ignored.
pub fn record_chain_verify(outcome: &str) {
    counter!("maidan_chain_verify_total", "outcome" => outcome.to_string()).increment(1);
}

/// A per-recipient notification written by the router, by the source event
/// `kind`. Deduped writes (a replay / a second replica) do not increment.
pub fn record_notification_created(kind: &str) {
    counter!("maidan_notifications_created_total", "kind" => kind.to_string()).increment(1);
}

/// A notification the router chose NOT to write, by `reason` — e.g. `muted`
/// when the recipient has muted the kind.
pub fn record_notification_suppressed(reason: &str) {
    counter!("maidan_notifications_suppressed_total", "reason" => reason.to_string()).increment(1);
}

/// A notification-email delivery attempt, by `outcome`:
/// - `sent` / `failed` — an immediate per-notification email.
/// - `skipped_present` — suppressed because the recipient was seen within the
///   presence window.
/// - `skipped_digest` — suppressed because the recipient is in digest mode;
///   the digest sweeper emails them instead.
/// - `skipped_withdrawn` — not queued because the message it is about was
///   withdrawn first.
/// - `digest` / `digest_failed` — a periodic digest rollup send.
///
/// Best-effort: a `failed` send is logged + counted, not retried (a
/// `digest_failed` leaves the watermark so the next sweep retries).
pub fn record_email_delivered(outcome: &str) {
    counter!("maidan_email_delivered_total", "outcome" => outcome.to_string()).increment(1);
}

/// Web Push delivery outcomes: `sent` / `failed` / `pruned` (subscription gone)
/// / `skipped_present` (member has a live WS) / `skipped_suspended` (the
/// member's workspace is suspended).
pub fn record_web_push_delivered(outcome: &str) {
    counter!("maidan_web_push_delivered_total", "outcome" => outcome.to_string()).increment(1);
}

/// Slack projector egress outcomes: `sent` / `failed`.
pub fn record_slack_egress(outcome: &str) {
    counter!("maidan_slack_egress_total", "outcome" => outcome.to_string()).increment(1);
}

/// GitHub projector egress outcomes: `sent` / `failed`.
pub fn record_github_egress(outcome: &str) {
    counter!("maidan_github_egress_total", "outcome" => outcome.to_string()).increment(1);
}

/// Inline pull-request review posts: `sent` (GitHub accepted the review),
/// `skipped` (no `head_sha`, no usable findings, or not `reviewed`), `failed`
/// (GitHub rejected; the 379 summary comment still landed).
pub fn record_github_review(outcome: &str) {
    counter!("maidan_github_review_total", "outcome" => outcome.to_string()).increment(1);
}

/// Mark-ready flips: `marked` (the draft is now ready), `already_ready` (a
/// retry found it ready), `refused` (the call failed a guard: not the mark-ready app,
/// bad input, or the PR failed the change-flow guards), `failed` (GitHub or
/// the store failed). Every terminal outcome of the endpoint is metered.
pub fn record_github_mark_ready(outcome: &str) {
    counter!("maidan_github_mark_ready_total", "outcome" => outcome.to_string()).increment(1);
}

/// A projector-egress delivery leaving the durable queue, by `surface` (`slack`
/// | `github` | `github_branch` | `linear` | `other`) and `outcome`:
/// - `sent` — posted to the external surface.
/// - `retry` — failed transiently; rescheduled with backoff (377.2).
/// - `dead` — dead-lettered after exhausting its attempts (377.2).
/// - `disabled` — an auth/config-class failure; the link was turned off and the
///   delivery dead-lettered without further retries (377.3).
/// - `unroutable` — the stored destination does not decode, so no sender can
///   address it; dead-lettered on sight (377.2).
/// - `refused` — the surface answered, and the result refused delivery.
/// - `over_allowance` — parked: the workspace is past its deliveries allowance.
///
/// The queue-level companion to `maidan_{slack,github}_egress_total`, which
/// stay a count of *post attempts* against each surface.
///
/// `surface` reads through the same closed list as
/// `maidan_result_deliveries_total` (an unknown stored string is `other`), and
/// `outcome` is an [`EgressOutcome`], so neither label can grow a new series.
pub fn record_egress_delivery(surface: &str, outcome: EgressOutcome) {
    counter!(
        "maidan_egress_deliveries_total",
        "surface" => delivery_surface(surface),
        "outcome" => outcome.as_str(),
    )
    .increment(1);
}

/// What happened to one egress delivery: the closed `outcome` label of
/// `maidan_egress_deliveries_total`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EgressOutcome {
    Sent,
    Retry,
    Dead,
    Disabled,
    Unroutable,
    Refused,
    OverAllowance,
}

impl EgressOutcome {
    pub const ALL: [EgressOutcome; 7] = [
        EgressOutcome::Sent,
        EgressOutcome::Retry,
        EgressOutcome::Dead,
        EgressOutcome::Disabled,
        EgressOutcome::Unroutable,
        EgressOutcome::Refused,
        EgressOutcome::OverAllowance,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            EgressOutcome::Sent => "sent",
            EgressOutcome::Retry => "retry",
            EgressOutcome::Dead => "dead",
            EgressOutcome::Disabled => "disabled",
            EgressOutcome::Unroutable => "unroutable",
            EgressOutcome::Refused => "refused",
            EgressOutcome::OverAllowance => "over_allowance",
        }
    }
}

/// A retry the per-host retry budget refused, by `worker` (`webhook`,
/// `automation`, `egress`, `mail`). The row was rescheduled a few seconds out
/// with its attempt count unchanged: a sustained rate means a destination is
/// taking retries as fast as the budget allows, not that deliveries are failing.
pub fn record_retry_deferred(worker: &str) {
    counter!("maidan_egress_retry_deferred_total", "worker" => worker.to_string()).increment(1);
}

/// Result-delivery trigger outcomes: `enqueued` (blessed, armed, sitting on the
/// egress outbox) or `skipped` (unknown surface, unusable target, or not on the
/// workspace allowlist). A skip is a recorded normal outcome, not an error.
/// A background loop started: both of its series exist from now on, so the
/// first panic reads as an increase from 0, and a first tick that never
/// finishes still ages the liveness gauge into a stall alert.
pub fn register_loop(name: &'static str) {
    counter!("maidan_background_loop_panics_total", "loop" => name).absolute(0);
    record_loop_tick(name);
}

/// A background loop finished a tick (panicked or not): its liveness.
pub fn record_loop_tick(name: &'static str) {
    gauge!("maidan_background_loop_last_tick_seconds", "loop" => name)
        .set(chrono::Utc::now().timestamp() as f64);
}

/// A background loop's tick panicked; the loop logged it and runs on.
pub fn record_loop_panic(name: &'static str) {
    counter!("maidan_background_loop_panics_total", "loop" => name).increment(1);
}

pub fn record_result_delivery(surface: &str, outcome: &str) {
    counter!(
        "maidan_result_deliveries_total",
        "surface" => delivery_surface(surface),
        "outcome" => outcome.to_string()
    )
    .increment(1);
}

/// A delivery surface as a closed label: a surface this build does not know
/// reads as `other`, so a stored string never becomes a series.
fn delivery_surface(surface: &str) -> &'static str {
    ["github", "github_branch", "slack", "linear"]
        .into_iter()
        .find(|known| *known == surface)
        .unwrap_or("other")
}

/// A task schedule fired by the scheduler sweeper. `outcome` is `created` when
/// the task thread was created, `failed` when creation errored.
pub fn record_task_schedule_fired(outcome: &str) {
    counter!("maidan_task_schedules_fired_total", "outcome" => outcome.to_string()).increment(1);
}

/// A thread wait timed out and the sweeper escalated, labeled by the escalation
/// policy applied (`notify` | `park`).
pub fn record_wait_timed_out(policy: &str) {
    counter!("maidan_wait_timed_out_total", "policy" => policy.to_string()).increment(1);
}

fn sync_hydrate_counters(current: HydrateSnapshot) {
    let mut guard = LAST_HYDRATE.lock().unwrap_or_else(PoisonError::into_inner);
    let last = guard.unwrap_or_default();
    increment_hydrate_delta("ok", current.ok, last.ok);
    increment_hydrate_delta("not_found", current.not_found, last.not_found);
    increment_hydrate_delta("failed", current.failed, last.failed);
    increment_hydrate_delta("backfilled", current.backfilled, last.backfilled);
    increment_hydrate_delta(
        "invalid_payload",
        current.invalid_payload,
        last.invalid_payload,
    );
    *guard = Some(current);
}

fn sync_read_routing_counters(current: (u64, u64)) {
    let mut guard = LAST_READ_ROUTING
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let (last_primary, last_replica) = *guard;
    let (primary, replica) = current;
    let dp = primary.saturating_sub(last_primary);
    if dp > 0 {
        counter!("maidan_replica_reads_total", "outcome" => "primary").increment(dp);
    }
    let dr = replica.saturating_sub(last_replica);
    if dr > 0 {
        counter!("maidan_replica_reads_total", "outcome" => "replica").increment(dr);
    }
    *guard = current;
}

fn sync_search_read_routing_counters(current: (u64, u64)) {
    let mut guard = LAST_SEARCH_READ_ROUTING
        .lock()
        .unwrap_or_else(PoisonError::into_inner);
    let (last_primary, last_replica) = *guard;
    let (primary, replica) = current;
    let dp = primary.saturating_sub(last_primary);
    if dp > 0 {
        counter!("maidan_search_replica_reads_total", "outcome" => "primary").increment(dp);
    }
    let dr = replica.saturating_sub(last_replica);
    if dr > 0 {
        counter!("maidan_search_replica_reads_total", "outcome" => "replica").increment(dr);
    }
    *guard = current;
}

fn increment_hydrate_delta(result: &str, current: u64, last: u64) {
    let delta = current.saturating_sub(last);
    if delta > 0 {
        counter!(
            "maidan_bus_notify_hydrate_total",
            "result" => result.to_string()
        )
        .increment(delta);
    }
}

async fn refresh_runtime_gauges(state: &AppState) {
    let ms = state.indexer_last_event_unix_ms.load(Ordering::Relaxed);
    let age_secs = if ms == 0 {
        0.0
    } else {
        ((Utc::now().timestamp_millis() - ms).max(0) as f64) / 1000.0
    };
    gauge!("maidan_indexer_last_event_age_seconds").set(age_secs);
    let pending_age_secs = match crate::health::indexer_backlog(state).await {
        Ok(Some(at)) if ms != 0 => ((Utc::now() - at).num_milliseconds().max(0) as f64) / 1000.0,
        _ => 0.0,
    };
    gauge!("maidan_indexer_pending_age_seconds").set(pending_age_secs);
    // Same guard as the age: before the indexer's first pass its watermark is
    // 0 and the whole log would read as behind.
    if ms == 0 {
        gauge!("maidan_indexer_events_behind").set(0.0);
    } else {
        match crate::health::indexer_events_behind(state).await {
            Ok(n) => gauge!("maidan_indexer_events_behind").set(n as f64),
            // A failed or slow count must not read as "caught up".
            Err(err) => tracing::warn!(
                error = %err,
                "metrics: indexer events-behind count failed; gauge left as it was"
            ),
        }
    }

    // Live embedding pipeline: queue depth is hard-capped by capacity, so the
    // lag this gauge reports is bounded.
    let im = &state.indexer_metrics;
    gauge!("maidan_indexer_queue_depth").set(im.queue_depth.load(Ordering::Relaxed) as f64);
    gauge!("maidan_indexer_queue_capacity").set(im.queue_capacity as f64);
    gauge!("maidan_indexer_embedded_total").set(im.embedded_total.load(Ordering::Relaxed) as f64);
    gauge!("maidan_indexer_embed_failed_total").set(im.failed_total.load(Ordering::Relaxed) as f64);
    gauge!("maidan_indexer_embed_batches_total")
        .set(im.batches_total.load(Ordering::Relaxed) as f64);
    gauge!("maidan_indexer_embed_retries_total")
        .set(im.retries_total.load(Ordering::Relaxed) as f64);
    gauge!("maidan_indexer_embed_repaired_total")
        .set(im.repaired_total.load(Ordering::Relaxed) as f64);

    if let Some(health) = state.bus_listener_health.as_ref() {
        let ok = health.check().is_ok();
        gauge!("maidan_bus_listener_ok").set(if ok { 1.0 } else { 0.0 });
        gauge!("maidan_bus_listener_errors_total").set(health.errors_total() as f64);
    }

    if let Some(stats) = state.bus_hydrate_stats.as_ref() {
        sync_hydrate_counters(stats.snapshot());
    }

    gauge!("maidan_ws_connections").set(state.ws_connections.load(Ordering::Relaxed) as f64);
    gauge!("maidan_http_in_flight_requests").set(state.request_limit.in_flight() as f64);
    gauge!("maidan_mcp_streamable_sessions").set(state.mcp.reap_streamable_sessions().await as f64);

    if let Some(routing) = state.read_routing_metrics.as_ref() {
        sync_read_routing_counters(routing.snapshot());
        gauge!("maidan_replica_lag_bytes").set(routing.lag_bytes() as f64);
    }

    if let Some(routing) = state.search_read_routing_metrics.as_ref() {
        sync_search_read_routing_counters(routing.snapshot());
    }

    if let Some(backend) = state.outbox_backend.as_ref() {
        if let Ok(pending) = crate::outbox_relay::pending_count(backend).await {
            gauge!("maidan_outbox_pending").set(pending as f64);
        }
        if let Ok(quarantined) = crate::outbox_relay::quarantined_count(backend).await {
            gauge!("maidan_outbox_quarantined").set(quarantined as f64);
        }
        if let Ok(Some(age)) = crate::outbox_relay::oldest_pending_age_secs(backend).await {
            gauge!("maidan_outbox_oldest_pending_seconds").set(age);
        } else {
            gauge!("maidan_outbox_oldest_pending_seconds").set(0.0);
        }
    }

    // The other two dead-letter queues. Both had a `count_dead_*` store method
    // on each backend and no gauge, so the only way to notice a filling DLQ was
    // to call the operator route and look — a projector delivery that has given
    // up on a tenant's Slack channel, or a notification email that will never
    // arrive, was invisible to alerting. The outbox has had
    // `maidan_outbox_pending` / `_quarantined` since 84.
    if let Ok(dead) = state.store.count_dead_egress().await {
        gauge!("maidan_egress_dead").set(dead as f64);
    }
    if let Ok(dead) = state.store.count_dead_mail().await {
        gauge!("maidan_mail_dead").set(dead as f64);
    }

    // A claim that only renews: the board marks such a thread stalled at
    // read time, and this is the same test counted, so it can page. With
    // detection off the gauge reads 0.
    let now = chrono::Utc::now();
    let stalled = match state
        .stall_threshold
        .and_then(|threshold| chrono::Duration::from_std(threshold).ok())
        .and_then(|threshold| now.checked_sub_signed(threshold))
    {
        Some(progress_before) => state
            .store
            .count_stalled_threads(now, progress_before)
            .await
            .inspect_err(|err| {
                tracing::warn!(error = %err, "metrics: stalled-thread count failed; gauge left as it was");
            })
            .ok(),
        None => Some(0),
    };
    if let Some(stalled) = stalled {
        gauge!("maidan_threads_stalled").set(stalled as f64);
    }
}

/// `GET /metrics` — Prometheus text exposition.
pub async fn scrape(State(state): State<AppState>) -> impl IntoResponse {
    refresh_runtime_gauges(&state).await;
    let body = PROMETHEUS
        .get()
        .map(|h| h.render())
        .unwrap_or_else(|| "# metrics not initialized\n".to_string());
    (StatusCode::OK, body)
}

/// Count requests and record latency by method + status class.
pub async fn middleware(request: Request, next: Next) -> Response {
    let method = request.method().as_str().to_string();
    let started = Instant::now();
    let response = next.run(request).await;
    let status = response.status().as_u16();
    let elapsed = started.elapsed().as_secs_f64();

    counter!(
        "http.server.request_total",
        "method" => method.clone(),
        "status" => status.to_string()
    )
    .increment(1);
    histogram!(
        "http.server.request_duration_seconds",
        "method" => method
    )
    .record(elapsed);

    response
}

#[cfg(test)]
mod egress_label_tests {
    use super::{delivery_surface, EgressOutcome};

    #[test]
    fn every_known_surface_keeps_its_own_label() {
        for surface in ["github", "github_branch", "slack", "linear"] {
            assert_eq!(delivery_surface(surface), surface);
        }
    }

    #[test]
    fn a_surface_this_build_does_not_know_reads_as_other() {
        for stored in ["teams", "", "Slack", "github ", "x\u{0}y"] {
            assert_eq!(delivery_surface(stored), "other", "{stored:?}");
        }
    }

    #[test]
    fn every_egress_surface_variant_has_a_closed_label() {
        use maidan_types::EgressSurface;
        for surface in [
            EgressSurface::Slack,
            EgressSurface::Github,
            EgressSurface::GithubBranch,
            EgressSurface::Linear,
        ] {
            assert_eq!(delivery_surface(surface.as_str()), surface.as_str());
        }
    }

    #[test]
    fn egress_outcomes_are_a_fixed_set_of_distinct_snake_case_labels() {
        let labels: Vec<&str> = EgressOutcome::ALL.iter().map(|o| o.as_str()).collect();
        assert_eq!(
            labels,
            [
                "sent",
                "retry",
                "dead",
                "disabled",
                "unroutable",
                "refused",
                "over_allowance"
            ]
        );
        let mut deduped = labels.clone();
        deduped.sort_unstable();
        deduped.dedup();
        assert_eq!(deduped.len(), labels.len());
    }
}

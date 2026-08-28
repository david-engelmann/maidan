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
use maidan_observability::{
    build_otlp_metrics_recorder, otlp_metrics_endpoint_from_env, otlp_metrics_interval_from_env,
    MeterGuard, MetricsPushConfig,
};
use metrics::{counter, describe_counter, describe_gauge, describe_histogram, gauge, histogram};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};
use metrics_util::layers::FanoutBuilder;

use crate::state::AppState;

static PROMETHEUS: OnceLock<PrometheusHandle> = OnceLock::new();
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

/// Install the global metrics recorder (idempotent).
pub fn init() {
    INIT.call_once(|| {
        let prom_recorder = PrometheusBuilder::new().build_recorder();
        let handle = prom_recorder.handle();

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
        describe_gauge!(
            "maidan_indexer_last_event_age_seconds",
            "Seconds since the background indexer last observed an event (0 if never)"
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
            "maidan_a2a_push_total",
            "A2A task push notifications by outcome (ok/failed after retries)"
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
        describe_histogram!(
            "maidan_automation_delivery_duration_seconds",
            "Automation HTTP delivery attempt latency"
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
/// on any non-zero rate (Cluster 184).
pub fn record_event_append_failure() {
    counter!("maidan_event_append_failures_total").increment(1);
}

/// Rows deleted by the retention sweeper for `table` (Cluster 186).
pub fn record_retention_pruned(table: &str, count: u64) {
    counter!("maidan_retention_pruned_total", "table" => table.to_string()).increment(count);
}

/// A per-recipient notification written by the router (Cluster 238), by the source
/// event `kind`. Deduped writes (a replay / a second replica) do not increment.
pub fn record_notification_created(kind: &str) {
    counter!("maidan_notifications_created_total", "kind" => kind.to_string()).increment(1);
}

/// A notification the router chose NOT to write (Cluster 242), by `reason` — e.g.
/// `muted` when the recipient has muted the kind.
pub fn record_notification_suppressed(reason: &str) {
    counter!("maidan_notifications_suppressed_total", "reason" => reason.to_string()).increment(1);
}

/// A notification-email delivery attempt, by `outcome`:
/// - `sent` / `failed` — an immediate per-notification email (Cluster 249).
/// - `skipped_present` — suppressed because the recipient was seen within the
///   presence window (Cluster 253).
/// - `skipped_digest` — suppressed because the recipient is in digest mode
///   (Cluster 255); the digest sweeper emails them instead.
/// - `digest` / `digest_failed` — a periodic digest rollup send (Cluster 255).
///
/// Best-effort: a `failed` send is logged + counted, not retried (a `digest_failed`
/// leaves the watermark so the next sweep retries).
pub fn record_email_delivered(outcome: &str) {
    counter!("maidan_email_delivered_total", "outcome" => outcome.to_string()).increment(1);
}

/// Slack projector egress outcomes (Cluster 309): `sent` / `failed`.
pub fn record_slack_egress(outcome: &str) {
    counter!("maidan_slack_egress_total", "outcome" => outcome.to_string()).increment(1);
}

/// GitHub projector egress outcomes (Cluster 312): `sent` / `failed`.
pub fn record_github_egress(outcome: &str) {
    counter!("maidan_github_egress_total", "outcome" => outcome.to_string()).increment(1);
}

/// A task schedule fired by the scheduler sweeper (Cluster 227). `outcome` is
/// `created` when the task thread was created, `failed` when creation errored.
pub fn record_task_schedule_fired(outcome: &str) {
    counter!("maidan_task_schedules_fired_total", "outcome" => outcome.to_string()).increment(1);
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

    // Live embedding pipeline (Cluster 116): queue depth is hard-capped by
    // capacity, so the lag this gauge reports is bounded.
    let im = &state.indexer_metrics;
    gauge!("maidan_indexer_queue_depth").set(im.queue_depth.load(Ordering::Relaxed) as f64);
    gauge!("maidan_indexer_queue_capacity").set(im.queue_capacity as f64);
    gauge!("maidan_indexer_embedded_total").set(im.embedded_total.load(Ordering::Relaxed) as f64);
    gauge!("maidan_indexer_embed_failed_total").set(im.failed_total.load(Ordering::Relaxed) as f64);
    gauge!("maidan_indexer_embed_batches_total")
        .set(im.batches_total.load(Ordering::Relaxed) as f64);

    if let Some(health) = state.bus_listener_health.as_ref() {
        let ok = health.check().is_ok();
        gauge!("maidan_bus_listener_ok").set(if ok { 1.0 } else { 0.0 });
        gauge!("maidan_bus_listener_errors_total").set(health.errors_total() as f64);
    }

    if let Some(stats) = state.bus_hydrate_stats.as_ref() {
        sync_hydrate_counters(stats.snapshot());
    }

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

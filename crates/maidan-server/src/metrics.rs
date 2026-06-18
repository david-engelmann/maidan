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

fn sync_hydrate_counters(current: HydrateSnapshot) {
    let mut guard = LAST_HYDRATE.lock().unwrap_or_else(PoisonError::into_inner);
    let last = guard.unwrap_or_default();
    increment_hydrate_delta("ok", current.ok, last.ok);
    increment_hydrate_delta("not_found", current.not_found, last.not_found);
    increment_hydrate_delta("failed", current.failed, last.failed);
    increment_hydrate_delta(
        "invalid_payload",
        current.invalid_payload,
        last.invalid_payload,
    );
    *guard = Some(current);
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

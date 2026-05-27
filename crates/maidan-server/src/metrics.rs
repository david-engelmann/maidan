//! Prometheus metrics (Track T.4).

use std::{
    sync::{atomic::Ordering, Once, OnceLock},
    time::Instant,
};

use axum::{
    extract::Request,
    extract::State,
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use chrono::Utc;
use metrics::{counter, describe_counter, describe_gauge, describe_histogram, gauge, histogram};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};

use crate::state::AppState;

static PROMETHEUS: OnceLock<PrometheusHandle> = OnceLock::new();
static INIT: Once = Once::new();

/// Install the global metrics recorder (idempotent).
pub fn init() {
    INIT.call_once(|| {
        let handle = PrometheusBuilder::new()
            .install_recorder()
            .expect("prometheus recorder");
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
    });
}

fn refresh_runtime_gauges(state: &AppState) {
    let ms = state.indexer_last_event_unix_ms.load(Ordering::Relaxed);
    let age_secs = if ms == 0 {
        0.0
    } else {
        ((Utc::now().timestamp_millis() - ms).max(0) as f64) / 1000.0
    };
    gauge!("maidan_indexer_last_event_age_seconds").set(age_secs);

    if let Some(health) = state.bus_listener_health.as_ref() {
        let ok = health.check().is_ok();
        gauge!("maidan_bus_listener_ok").set(if ok { 1.0 } else { 0.0 });
        gauge!("maidan_bus_listener_errors_total").set(health.errors_total() as f64);
    }
}

/// `GET /metrics` — Prometheus text exposition.
pub async fn scrape(State(state): State<AppState>) -> impl IntoResponse {
    refresh_runtime_gauges(&state);
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

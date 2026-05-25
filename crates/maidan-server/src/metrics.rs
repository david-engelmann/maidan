//! Prometheus metrics (Track T.4).

use std::{
    sync::{Once, OnceLock},
    time::Instant,
};

use axum::{
    extract::Request,
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use metrics::{counter, describe_counter, describe_histogram, histogram};
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusHandle};

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
    });
}

/// `GET /metrics` — Prometheus text exposition.
pub async fn scrape() -> impl IntoResponse {
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

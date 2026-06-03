//! OTLP metrics push (`metrics` crate → OpenTelemetry SDK).

use std::time::Duration;

use metrics_exporter_opentelemetry::Recorder;
use opentelemetry::KeyValue;
use opentelemetry_otlp::{MetricExporter, WithExportConfig};
use opentelemetry_sdk::metrics::{PeriodicReader, SdkMeterProvider};
use opentelemetry_sdk::Resource;

use crate::InitError;

/// Push interval and OTLP endpoint for metrics export.
#[derive(Debug, Clone)]
pub struct MetricsPushConfig {
    pub service_name: String,
    pub endpoint: String,
    pub interval: Duration,
}

/// Keeps the SDK meter provider alive until [`MeterGuard::shutdown`].
pub struct MeterGuard(SdkMeterProvider);

impl MeterGuard {
    pub fn shutdown(self) {
        if let Err(err) = self.0.shutdown() {
            eprintln!("opentelemetry metrics shutdown error: {err}");
        }
    }
}

fn env_truthy(name: &str) -> bool {
    std::env::var(name)
        .map(|v| {
            matches!(
                v.trim().to_ascii_lowercase().as_str(),
                "1" | "true" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

/// Resolved OTLP metrics endpoint when push export is enabled.
pub fn otlp_metrics_endpoint_from_env() -> Option<String> {
    let dedicated = std::env::var("OTLP_METRICS_ENDPOINT")
        .ok()
        .filter(|s| !s.trim().is_empty());
    if dedicated.is_some() {
        return dedicated;
    }
    if !env_truthy("OTLP_METRICS") {
        return None;
    }
    std::env::var("OTLP_ENDPOINT")
        .ok()
        .filter(|s| !s.trim().is_empty())
}

/// Export interval for periodic OTLP metrics push (default 15s).
pub fn otlp_metrics_interval_from_env() -> Duration {
    std::env::var("OTLP_METRICS_INTERVAL_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|&secs| secs > 0)
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(15))
}

/// Build an OpenTelemetry [`Recorder`] backed by a periodic OTLP push exporter.
pub fn build_otlp_metrics_recorder(
    config: &MetricsPushConfig,
) -> Result<(MeterGuard, Recorder), InitError> {
    let exporter = MetricExporter::builder()
        .with_tonic()
        .with_endpoint(config.endpoint.clone())
        .with_timeout(Duration::from_secs(3))
        .build()
        .map_err(|e| InitError::Otlp(e.to_string()))?;

    let resource = Resource::builder()
        .with_attributes([KeyValue::new("service.name", config.service_name.clone())])
        .build();

    let reader = PeriodicReader::builder(exporter)
        .with_interval(config.interval)
        .build();

    let (provider, recorder) = Recorder::builder("maidan-server")
        .with_meter_provider(|builder| builder.with_resource(resource).with_reader(reader))
        .build();

    Ok((MeterGuard(provider), recorder))
}

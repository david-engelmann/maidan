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

/// Whether a raw env value is truthy (`1`/`true`/`yes`/`on`, case-insensitive).
fn is_truthy(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

fn env_truthy(name: &str) -> bool {
    std::env::var(name).map(|v| is_truthy(&v)).unwrap_or(false)
}

/// Pure resolution of the OTLP metrics endpoint from raw inputs (see
/// [`otlp_metrics_endpoint_from_env`]): a dedicated endpoint wins; otherwise the
/// shared OTLP endpoint is used only when metrics push is enabled. Blank strings
/// count as unset.
fn resolve_metrics_endpoint(
    dedicated: Option<String>,
    metrics_enabled: bool,
    otlp_endpoint: Option<String>,
) -> Option<String> {
    if let Some(dedicated) = dedicated.filter(|s| !s.trim().is_empty()) {
        return Some(dedicated);
    }
    if !metrics_enabled {
        return None;
    }
    otlp_endpoint.filter(|s| !s.trim().is_empty())
}

/// Pure parse of the metrics push interval (see [`otlp_metrics_interval_from_env`]):
/// a positive integer number of seconds, else the 15s default.
fn parse_metrics_interval(raw: Option<String>) -> Duration {
    raw.and_then(|s| s.parse().ok())
        .filter(|&secs| secs > 0)
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(15))
}

/// Resolved OTLP metrics endpoint when push export is enabled.
pub fn otlp_metrics_endpoint_from_env() -> Option<String> {
    resolve_metrics_endpoint(
        std::env::var("OTLP_METRICS_ENDPOINT").ok(),
        env_truthy("OTLP_METRICS"),
        std::env::var("OTLP_ENDPOINT").ok(),
    )
}

/// Export interval for periodic OTLP metrics push (default 15s).
pub fn otlp_metrics_interval_from_env() -> Duration {
    parse_metrics_interval(std::env::var("OTLP_METRICS_INTERVAL_SECS").ok())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truthy_values_are_case_insensitive_and_trimmed() {
        for v in ["1", "true", "TRUE", " Yes ", "on"] {
            assert!(is_truthy(v), "{v:?} should be truthy");
        }
        for v in ["0", "false", "", "off", "no", "2"] {
            assert!(!is_truthy(v), "{v:?} should not be truthy");
        }
    }

    #[test]
    fn metrics_interval_parses_positive_seconds_else_defaults() {
        assert_eq!(
            parse_metrics_interval(Some("5".into())),
            Duration::from_secs(5)
        );
        // Non-positive, unparseable, and unset all fall back to 15s.
        assert_eq!(
            parse_metrics_interval(Some("0".into())),
            Duration::from_secs(15)
        );
        assert_eq!(
            parse_metrics_interval(Some("nope".into())),
            Duration::from_secs(15)
        );
        assert_eq!(parse_metrics_interval(None), Duration::from_secs(15));
    }

    #[test]
    fn metrics_endpoint_prefers_dedicated_then_gated_shared() {
        // Dedicated endpoint always wins, even if metrics push is off.
        assert_eq!(
            resolve_metrics_endpoint(Some("http://d".into()), false, Some("http://o".into())),
            Some("http://d".into())
        );
        // No dedicated + metrics on → shared endpoint.
        assert_eq!(
            resolve_metrics_endpoint(None, true, Some("http://o".into())),
            Some("http://o".into())
        );
        // No dedicated + metrics off → none, even with a shared endpoint set.
        assert_eq!(
            resolve_metrics_endpoint(None, false, Some("http://o".into())),
            None
        );
        // Blank strings count as unset.
        assert_eq!(
            resolve_metrics_endpoint(Some("  ".into()), true, Some("http://o".into())),
            Some("http://o".into())
        );
        assert_eq!(resolve_metrics_endpoint(None, true, Some("".into())), None);
    }
}

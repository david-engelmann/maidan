//! OpenTelemetry and [`tracing`] setup for Maidan binaries.
//!
//! Call [`init`] once at process startup; keep the returned [`Guard`] alive
//! until shutdown, then call [`Guard::shutdown`].

mod metrics;

#[cfg(feature = "otel")]
use std::time::Duration;

#[cfg(feature = "otel")]
use opentelemetry::trace::TracerProvider as OtelTracerProvider;
#[cfg(feature = "otel")]
use opentelemetry::KeyValue;
#[cfg(feature = "otel")]
use opentelemetry_otlp::{SpanExporter, WithExportConfig};
#[cfg(feature = "otel")]
use opentelemetry_sdk::trace::SdkTracerProvider;
#[cfg(feature = "otel")]
use opentelemetry_sdk::Resource;
use thiserror::Error;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer as _};

#[cfg(feature = "otel")]
pub use metrics::{build_otlp_metrics_recorder, MeterGuard};
pub use metrics::{
    otlp_metrics_endpoint_from_env, otlp_metrics_interval_from_env, MetricsPushConfig,
};

/// Log output format.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFormat {
    Plain,
    Json,
}

/// Configuration loaded from environment variables.
#[derive(Debug, Clone)]
pub struct Config {
    pub log_filter: String,
    pub log_format: LogFormat,
    pub otlp_endpoint: Option<String>,
    pub service_name: String,
}

/// Parse the `MAIDAN_LOG_FORMAT` value: `json` (case-insensitive) → JSON, any
/// other value (including unset) → Plain.
fn parse_log_format(raw: &str) -> LogFormat {
    match raw.trim().to_ascii_lowercase().as_str() {
        "json" => LogFormat::Json,
        _ => LogFormat::Plain,
    }
}

impl Config {
    pub fn from_env() -> Self {
        let log_filter =
            std::env::var("MAIDAN_LOG").unwrap_or_else(|_| "info,sqlx=warn".to_string());
        let log_format = parse_log_format(
            &std::env::var("MAIDAN_LOG_FORMAT").unwrap_or_else(|_| "plain".into()),
        );
        let otlp_endpoint = std::env::var("OTLP_ENDPOINT")
            .ok()
            .filter(|s| !s.trim().is_empty());
        let service_name =
            std::env::var("OTLP_SERVICE_NAME").unwrap_or_else(|_| "maidan-server".to_string());
        Self {
            log_filter,
            log_format,
            otlp_endpoint,
            service_name,
        }
    }
}

#[derive(Debug, Error)]
pub enum InitError {
    #[error("failed to build OTLP exporter: {0}")]
    Otlp(String),

    #[error("tracing subscriber already initialized")]
    AlreadyInitialized,
}

/// Handle keeping OTel providers alive until [`Guard::shutdown`]. When the `otel`
/// feature is compiled out this carries nothing and `shutdown` is a no-op.
pub struct Guard {
    #[cfg(feature = "otel")]
    tracer_provider: Option<SdkTracerProvider>,
    #[cfg(feature = "otel")]
    meter_provider: Option<metrics::MeterGuard>,
}

impl Guard {
    pub fn shutdown(self) {
        #[cfg(feature = "otel")]
        {
            if let Some(provider) = self.meter_provider {
                provider.shutdown();
            }
            if let Some(provider) = self.tracer_provider {
                if let Err(err) = provider.shutdown() {
                    eprintln!("opentelemetry trace shutdown error: {err}");
                }
            }
        }
    }
}

/// Initialize global `tracing` + optional OTLP trace export. With the `otel`
/// feature off, only plain `tracing` is installed (OTLP is compiled out); an
/// `OTLP_ENDPOINT` set in that build is reported and otherwise ignored.
pub fn init(config: Config) -> Result<Guard, InitError> {
    let filter = EnvFilter::try_new(&config.log_filter).unwrap_or_else(|_| EnvFilter::new("info"));

    let fmt_layer = match config.log_format {
        LogFormat::Plain => tracing_subscriber::fmt::layer().with_target(false).boxed(),
        LogFormat::Json => tracing_subscriber::fmt::layer()
            .json()
            .with_target(false)
            .boxed(),
    };

    let registry = tracing_subscriber::registry().with(filter).with(fmt_layer);

    #[cfg(feature = "otel")]
    let guard = {
        let mut tracer_provider = None;
        if let Some(endpoint) = config.otlp_endpoint {
            let exporter = SpanExporter::builder()
                .with_tonic()
                .with_endpoint(endpoint)
                .with_timeout(Duration::from_secs(3))
                .build()
                .map_err(|e| InitError::Otlp(e.to_string()))?;

            let resource = Resource::builder()
                .with_attributes([KeyValue::new("service.name", config.service_name.clone())])
                .build();

            let provider = SdkTracerProvider::builder()
                .with_batch_exporter(exporter)
                .with_resource(resource)
                .build();

            let tracer = OtelTracerProvider::tracer(&provider, "maidan");
            let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);
            registry
                .with(otel_layer)
                .try_init()
                .map_err(|_| InitError::AlreadyInitialized)?;
            tracer_provider = Some(provider);
        } else {
            registry
                .try_init()
                .map_err(|_| InitError::AlreadyInitialized)?;
        }
        Guard {
            tracer_provider,
            meter_provider: None,
        }
    };

    #[cfg(not(feature = "otel"))]
    let guard = {
        if config.otlp_endpoint.is_some() {
            eprintln!(
                "OTLP_ENDPOINT is set but this build was compiled without the `otel` feature; \
                 OTLP trace export is disabled (plain tracing + Prometheus scrape unaffected)"
            );
        }
        registry
            .try_init()
            .map_err(|_| InitError::AlreadyInitialized)?;
        Guard {}
    };

    Ok(guard)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_format_parses_json_case_insensitively_else_plain() {
        assert_eq!(parse_log_format("json"), LogFormat::Json);
        assert_eq!(parse_log_format(" JSON "), LogFormat::Json);
        assert_eq!(parse_log_format("plain"), LogFormat::Plain);
        assert_eq!(parse_log_format(""), LogFormat::Plain);
        assert_eq!(parse_log_format("text"), LogFormat::Plain);
    }
}

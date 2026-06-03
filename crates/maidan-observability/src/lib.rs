//! OpenTelemetry and [`tracing`] setup for Maidan binaries.
//!
//! Call [`init`] once at process startup; keep the returned [`Guard`] alive
//! until shutdown, then call [`Guard::shutdown`].

mod metrics;

use std::time::Duration;

use opentelemetry::trace::TracerProvider as OtelTracerProvider;
use opentelemetry::KeyValue;
use opentelemetry_otlp::{SpanExporter, WithExportConfig};
use opentelemetry_sdk::trace::SdkTracerProvider;
use opentelemetry_sdk::Resource;
use thiserror::Error;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer as _};

pub use metrics::{
    build_otlp_metrics_recorder, otlp_metrics_endpoint_from_env, otlp_metrics_interval_from_env,
    MeterGuard, MetricsPushConfig,
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

impl Config {
    pub fn from_env() -> Self {
        let log_filter =
            std::env::var("MAIDAN_LOG").unwrap_or_else(|_| "info,sqlx=warn".to_string());
        let log_format = match std::env::var("MAIDAN_LOG_FORMAT")
            .unwrap_or_else(|_| "plain".into())
            .to_ascii_lowercase()
            .as_str()
        {
            "json" => LogFormat::Json,
            _ => LogFormat::Plain,
        };
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

/// Handle keeping OTel providers alive until [`Guard::shutdown`].
pub struct Guard {
    tracer_provider: Option<SdkTracerProvider>,
    meter_provider: Option<metrics::MeterGuard>,
}

impl Guard {
    pub fn shutdown(mut self) {
        if let Some(provider) = self.meter_provider.take() {
            provider.shutdown();
        }
        if let Some(provider) = self.tracer_provider.take() {
            if let Err(err) = provider.shutdown() {
                eprintln!("opentelemetry trace shutdown error: {err}");
            }
        }
    }
}

/// Initialize global `tracing` + optional OTLP trace export.
pub fn init(config: Config) -> Result<Guard, InitError> {
    let filter = EnvFilter::try_new(&config.log_filter).unwrap_or_else(|_| EnvFilter::new("info"));

    let fmt_layer = match config.log_format {
        LogFormat::Plain => tracing_subscriber::fmt::layer().with_target(false).boxed(),
        LogFormat::Json => tracing_subscriber::fmt::layer()
            .json()
            .with_target(false)
            .boxed(),
    };

    let mut tracer_provider = None;

    let registry = tracing_subscriber::registry().with(filter).with(fmt_layer);

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

    Ok(Guard {
        tracer_provider,
        meter_provider: None,
    })
}

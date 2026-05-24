//! OpenTelemetry and [`tracing`] setup for Maidan binaries.
//!
//! Call [`init`] once at process startup; keep the returned [`Guard`] alive
//! until shutdown, then call [`Guard::shutdown`].

use std::time::Duration;

use opentelemetry::trace::TracerProvider as OtelTracerProvider;
use opentelemetry::KeyValue;
use opentelemetry_otlp::{SpanExporter, WithExportConfig};
use opentelemetry_sdk::{runtime::Tokio, trace::TracerProvider, Resource};
use thiserror::Error;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter, Layer as _};

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

/// Handle keeping the OTel provider alive until [`Guard::shutdown`].
pub struct Guard {
    provider: Option<TracerProvider>,
}

impl Guard {
    pub fn shutdown(mut self) {
        if let Some(provider) = self.provider.take() {
            if let Err(err) = provider.shutdown() {
                eprintln!("opentelemetry shutdown error: {err}");
            }
        }
    }
}

/// Initialize global `tracing` + optional OTLP export.
pub fn init(config: Config) -> Result<Guard, InitError> {
    let filter = EnvFilter::try_new(&config.log_filter).unwrap_or_else(|_| EnvFilter::new("info"));

    let fmt_layer = match config.log_format {
        LogFormat::Plain => tracing_subscriber::fmt::layer().with_target(false).boxed(),
        LogFormat::Json => tracing_subscriber::fmt::layer()
            .json()
            .with_target(false)
            .boxed(),
    };

    let mut provider = None;

    let registry = tracing_subscriber::registry().with(filter).with(fmt_layer);

    if let Some(endpoint) = config.otlp_endpoint {
        let exporter = SpanExporter::builder()
            .with_tonic()
            .with_endpoint(endpoint)
            .with_timeout(Duration::from_secs(3))
            .build()
            .map_err(|e| InitError::Otlp(e.to_string()))?;

        let resource = Resource::new([KeyValue::new("service.name", config.service_name.clone())]);

        let tracer_provider = TracerProvider::builder()
            .with_batch_exporter(exporter, Tokio)
            .with_resource(resource)
            .build();

        let tracer = OtelTracerProvider::tracer(&tracer_provider, "maidan");
        let otel_layer = tracing_opentelemetry::layer().with_tracer(tracer);
        registry
            .with(otel_layer)
            .try_init()
            .map_err(|_| InitError::AlreadyInitialized)?;
        provider = Some(tracer_provider);
    } else {
        registry
            .try_init()
            .map_err(|_| InitError::AlreadyInitialized)?;
    }

    Ok(Guard { provider })
}

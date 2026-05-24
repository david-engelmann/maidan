//! Observability init is idempotent across separate processes only; this
//! test verifies plain init in-process once.

use maidan_observability::{init, Config, LogFormat};

#[test]
fn init_plain_without_otlp() {
    let guard = init(Config {
        log_filter: "warn".into(),
        log_format: LogFormat::Plain,
        otlp_endpoint: None,
        service_name: "maidan-test".into(),
    })
    .expect("init");
    tracing::warn!("observability smoke");
    guard.shutdown();
}

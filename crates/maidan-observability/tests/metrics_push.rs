//! OTLP metrics recorder wiring (stdout exporter; no network).

use std::time::Duration;

use metrics_exporter_opentelemetry::Recorder;
use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader};
use opentelemetry_sdk::Resource;

#[test]
fn build_otlp_metrics_recorder_exports_counter() {
    let exporter = InMemoryMetricExporter::default();
    let reader = PeriodicReader::builder(exporter.clone())
        .with_interval(Duration::from_millis(50))
        .build();

    let resource = Resource::builder()
        .with_attributes([opentelemetry::KeyValue::new("service.name", "maidan-test")])
        .build();

    let (provider, recorder) = Recorder::builder("maidan-test")
        .with_meter_provider(|builder| builder.with_resource(resource).with_reader(reader))
        .build();

    metrics::set_global_recorder(recorder).expect("set recorder");
    metrics::counter!("maidan_test_counter").increment(1);
    provider.force_flush().expect("flush");

    let finished = exporter.get_finished_metrics().expect("finished metrics");
    assert!(
        !finished.is_empty(),
        "expected at least one metric resource after flush"
    );

    let _ = provider.shutdown();
}

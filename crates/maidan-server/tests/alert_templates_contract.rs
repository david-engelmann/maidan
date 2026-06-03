//! Cluster 90: alert templates reference metrics the server exports.

use std::path::PathBuf;

#[test]
fn prometheus_slo_rules_reference_exported_metrics() {
    let rules = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/alerts/prometheus-rules-maidan-slo.yaml");
    let body = std::fs::read_to_string(&rules).expect("read rules yaml");

    let expected = [
        "http_server_request_duration_seconds",
        "maidan_automation_delivery_total",
        "maidan_outbox_pending",
        "maidan_outbox_oldest_pending_seconds",
        "maidan_outbox_quarantined",
        "maidan_bus_listener_ok",
        "maidan_indexer_last_event_age_seconds",
        "maidan_subscribe_replay_total",
    ];
    for metric in expected {
        assert!(
            body.contains(metric),
            "rules file should reference metric {metric}"
        );
    }
}

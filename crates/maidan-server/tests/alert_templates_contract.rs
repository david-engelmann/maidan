//! Alert templates reference metrics the server exports.

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
        // Batched-embed indexer gauges.
        "maidan_indexer_queue_depth",
        "maidan_indexer_queue_capacity",
        "maidan_indexer_embed_failed_total",
        // The other two dead-letter queues. Both had a `count_dead_*` store
        // method and no gauge, so a projector delivery that had given up on a
        // tenant's Slack channel — or a notification email that would never
        // arrive — was invisible to alerting, while the outbox has been
        // alertable for far longer.
        "maidan_egress_dead",
        "maidan_mail_dead",
        "maidan_authorization_decisions_total",
        // The permanent record: a change that committed without its audit row
        // or its event.
        "maidan_audit_write_failures_total",
        "maidan_event_append_failures_total",
    ];
    for metric in expected {
        assert!(
            body.contains(metric),
            "rules file should reference metric {metric}"
        );
    }
}

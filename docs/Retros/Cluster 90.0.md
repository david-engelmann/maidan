# Cluster 90.0 retro — SLO alert templates

> Tag **`v90.0.0`**.

## What shipped

- `docs/alerts/prometheus-rules-maidan-slo.yaml` — PrometheusRule for HTTP p99, automation failures, outbox lag/quarantine, bus listener, indexer staleness, subscribe replay truncation.
- `docs/alerts/alertmanager-routes.example.yaml` and `docs/alerts/README.md`.
- `scripts/validate-prometheus-rules.sh`; `alert_templates_contract` test.
- Production runbook links; corrected `maidan_automation_delivery_total{outcome="failure"}` label in docs.

## What was deferred

- PagerDuty receiver wiring (example routes only).
- Per-workspace automation DLQ depth gauge (documented proxy alerts).

## Next

Cluster **91** — bootstrap compile-time strip ([[Clusters/Product Ladder 77+]]).

# Cluster 89.0 retro — OTLP metrics export

> Tag **`v89.0.0`**.

## What shipped

- OpenTelemetry 0.31 in `maidan-observability`; optional OTLP metrics push (`OTLP_METRICS`, `OTLP_METRICS_ENDPOINT`).
- Prometheus fanout: `GET /metrics` unchanged when OTLP push is enabled.
- Example dashboard `docs/dashboards/maidan-operator.json`; otel Helm profile sets `OTLP_METRICS=1`.
- `metrics_push` integration test (in-memory exporter); Prometheus upkeep on std thread for unit tests.

## What was deferred

- Hosted Grafana Cloud bundle.
- Per-tool MCP latency series on `/metrics`.

## Next

Cluster **90** — SLO alert templates ([[Clusters/Product Ladder 77+]]).

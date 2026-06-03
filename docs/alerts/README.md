# SLO alert templates (`v90.0.0`)

Copy-paste starting points for Prometheus / Grafana Alerting / Alertmanager.
Metrics come from `GET /metrics` on `maidan-server` (see [[Production]]).

## Files

| File | Purpose |
|------|---------|
| `prometheus-rules-maidan-slo.yaml` | `PrometheusRule` / ruler groups: HTTP latency, automation failures, outbox lag |
| `alertmanager-routes.example.yaml` | Example route tree (wire your receivers) |

## Install (Prometheus Operator)

```bash
kubectl apply -f prometheus-rules-maidan-slo.yaml -n observability
```

Set `release: prometheus` (or your operator's `ruleSelector`) to match your
`Prometheus` CR. Tune thresholds via the `maidan_slo_*` recording-rule inputs
or edit `for:` / `expr` directly.

## Grafana

Import [[../dashboards/maidan-operator.json]] for visualization. Create alert
rules from the same PromQL as `prometheus-rules-maidan-slo.yaml`, or use
Grafana Mimir/Cortex ruler with the YAML groups.

## Automation DLQ depth

There is no per-workspace DLQ gauge on `/metrics` (fixed cardinality). Alerts
here use **`maidan_automation_delivery_total{outcome="failure"}`** rate and
**`maidan_outbox_quarantined`** as proxies. For exact quarantined row counts,
add a custom exporter against `GET /workspaces/:wid/automation/dlq` or SQL on
`maidan_automation_deliveries` where `quarantined_at IS NOT NULL`.

## Validate locally

```bash
./scripts/validate-prometheus-rules.sh
```

Uses `promtool` when installed; otherwise checks YAML structure and required
metric name substrings.

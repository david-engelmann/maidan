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

## Background loops

A background loop that stops, or keeps panicking, fails silently: a request
still answers while the work behind it stands still. Each loop sets
`maidan_background_loop_last_tick_seconds{loop}` when it starts and whenever
it finishes a tick, and counts `maidan_background_loop_panics_total{loop}`
(registered at 0 when it starts) when a tick panics; the loop logs the panic
at ERROR and runs its next tick. A loop turned off exports neither. The change sweep reports them
today.

- **`MaidanChangeSweepStalled`** (critical): no sweep tick finished for 6
  minutes, three default intervals, held for 5. Merges settling, change
  notices, handoffs and Linear claim comments have stopped. Read the sweep's
  last log lines, and restart the server if the task is gone.
- **`MaidanBackgroundLoopPanicking`** (warning): a tick panicked in the last
  15 minutes. The loop runs on, but the rest of that tick was skipped, and a
  panic that repeats skips it every tick. The message is in the log at ERROR.

These reach a person only through an alert route. A stack that scrapes no
`/metrics` gets neither: wire Alertmanager (`alertmanager-routes.example.yaml`)
or Grafana alerting before relying on them.

## Validate locally

```bash
./scripts/check-alert-rules.sh
```

Runs `promtool check rules` + `promtool test rules` (the unit tests in
`prometheus-rules-maidan-slo.test.yaml`) after extracting the raw rule groups
from the `PrometheusRule` CRD. Requires `promtool` (skips with a hint if it's
not installed); CI enforces it via the required `promtool (alert rules)` job.
Metric-name presence is additionally guarded by the `alert_templates_contract`
test (`cargo test -p maidan-server`).

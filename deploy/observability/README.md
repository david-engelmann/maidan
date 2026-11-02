# Observability

`maidan-swarm-dashboard.json` is a Grafana dashboard (schema 39) for a maidan
server scraped by Prometheus. The `job` variable (default `maidan-server`) picks the scrape job. Import it through
**Dashboards → New → Import** and pick your Prometheus data source.

| Panel | Metrics |
| --- | --- |
| Throughput | `maidan_thread_results_total` by harness and kind |
| Verdict mix | `maidan_thread_results_total` by harness and verdict |
| Cost | `maidan_usage_usd_micros_total`, `maidan_usage_tokens_total`, cache savings |
| Claim latency | `maidan_thread_claim_duration_seconds` (acknowledgement to result), reaped and unacknowledged claims |
| Delivery | `maidan_result_deliveries_total` and `maidan_egress_deliveries_total` by surface and outcome |

Every label on these metrics is a closed set. Thread, member and workspace ids
are never labels; they live on the `maidan.thread` spans, one trace per thread.

Usage carries no seat label, so "Dollars per stored result (all seats)" is a swarm-wide average. Per-seat cost belongs in the usage ledger and spans, not a metric label.

## A local workbench with Grafana Alloy

[`alloy/config.alloy`](alloy/config.alloy) takes maidan-server's OTLP push and
scrapes its `/metrics`, so a review run shows up as a trace and the usage
counters (`maidan_usage_tokens_total`, `maidan_usage_usd_micros_total`) land
in Prometheus. Point the server at it:

```sh
OTLP_ENDPOINT=http://alloy:4317 OTLP_METRICS=1
```

Alloy reads `MAIDAN_METRICS_TARGET` (default `maidan:8080`),
`TRACES_OTLP_ENDPOINT` (default `tempo:4317`) and `PROM_REMOTE_WRITE_URL`
(default `http://prometheus:9090/api/v1/write`). Before export it deletes any
`prompt`, `diff`, `rendered`, `body` or `content` span attribute, so no prompt or
diff text leaves the workbench even if a span records one; `scripts/otlp-smoke.sh`
also fails if the server exports one. Postgres stays the record: the traces and
metrics are a view of it, and losing them loses nothing.

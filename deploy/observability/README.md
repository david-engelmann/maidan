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

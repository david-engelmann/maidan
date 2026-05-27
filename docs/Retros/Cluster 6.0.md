# Cluster 6.0 retro — Delivery reliability

> Closing wave for Cluster 6.0 · target tag `v6.0.0`.

Cluster 6.0 made subscribe recovery and background task health observable:
Prometheus now reports lag/replay outcomes, indexer age, and Postgres listener
status, with operator runbooks wired to those signals.

## What shipped

- **PR #158** — Cluster kickoff plan ([[Clusters/Cluster 6.0]]).
- **PR #159** — Implementation bundle (6.0.1–6.0.4):
  - Subscribe-path metrics in shared event streaming (WS + MCP SSE):
    `maidan_bus_lag_total`, `maidan_bus_lag_skipped`,
    `maidan_subscribe_replay_total{outcome}`.
  - Runtime gauges on `/metrics`:
    `maidan_indexer_last_event_age_seconds`, `maidan_bus_listener_ok`,
    `maidan_bus_listener_errors_total`.
  - Listener health tracks cumulative errors (`errors_total`) for trend alerts.
  - Compose full profile sets `INDEXER_STALE_SECS=300`.
  - Production/Operations/Architecture docs include alerting and troubleshooting.

## What was deferred

| To         | What                                              | Why                                      |
|------------|---------------------------------------------------|------------------------------------------|
| Cluster 7+ | At-most-once bus semantics hardening              | 6.0 focused on observability, not protocol change. |
| Cluster 7+ | Per-model embedding tables / SQLite semantic      | Search-scope work, still out of 6.0.     |
| Cluster B  | SSE for MCP `resources/subscribe`                 | Long-standing deferral.                  |
| Post-6.0   | Coverage floor toward 11%+                        | Separate CI-focused wave.                |

## Surprises

- The existing replay-hint e2e was a good anchor for metrics assertions; we only
  needed to scrape `/metrics` after inducing lag.
- Listener-health state already encoded most of what operators needed; adding a
  monotonic error counter unlocked trend-based alerting with minimal code.
- One implementation PR remained manageable because all changes shared the same
  operational theme.

## Decisions

- **Metrics complement health** — `/health` remains readiness truth; `/metrics`
  adds trend visibility and transport/outcome granularity.
- **Fixed-cardinality labels only** — no workspace UUID labels to avoid series
  explosion under multi-tenant load.
- **Staleness default unchanged** — `INDEXER_STALE_SECS` stays opt-in (`0`), with
  production recommendation documented instead of forced globally.

## Capability table extension

| Capability                                              | First available in |
|---------------------------------------------------------|--------------------|
| Subscribe lag + replay Prometheus metrics (WS + MCP SSE) | `v6.0.0`         |
| Indexer age gauge on `/metrics`                         | `v6.0.0`           |
| Postgres listener health/error gauges on `/metrics`     | `v6.0.0`           |
| Delivery reliability alerting runbook                   | `v6.0.0`           |

## Risks identified + mitigated

- **Invisible subscriber lag** — now visible via counters/histogram plus replay outcomes.
- **Silent listener degradation** — now visible on both readiness and metrics.
- **Indexer silence ambiguity** — explicit age gauge + `INDEXER_STALE_SECS` guidance.

## Risks identified + still open

- **At-most-once delivery semantics** — observability improved; semantics unchanged.
- **Coverage depth** — floor still 10.0%; no bump in this wave.
- **SQLite semantic search** — still unsupported.

## Forward look

Next wave is open: either incremental coverage toward 11%+, or reliability hardening
beyond observability (e.g., stronger recovery semantics). See [[Open Work]].

## Acknowledgements

Solo cluster. Kickoff #158, implementation #159, this retro.

# Cluster 8.0 retro — Bus hydrate observability

> Closing wave for Cluster 8.0 · target tag `v8.0.0`.

Cluster 8.0 closed the operator loop on Postgres pointer delivery: hydrate
outcomes are visible on `/metrics` and documented for alerting, without
changing at-most-once NOTIFY semantics.

## What shipped

- **PR #164** — Cluster kickoff plan ([[Clusters/Cluster 8.0]]).
- **PR #165** — Implementation bundle (8.0.1–8.0.3):
  - `HydrateStats` in `maidan-bus` with `maidan_bus_notify_hydrate_total{result}`
    (`ok`, `not_found`, `failed`, `invalid_payload`) exported on `/metrics` scrape.
  - `AppState.bus_hydrate_stats` wired for Postgres deployments only.
  - Production/Operations/Architecture hydrate alerting and troubleshooting.
  - Integration tests for `ok` and `not_found` hydrate counters.

## What was deferred

| To         | What                                              | Why                                      |
|------------|---------------------------------------------------|------------------------------------------|
| Post-8.0   | Outbox / at-least-once bus semantics              | Observability only; protocol unchanged.  |
| Post-8.0   | Coverage floor toward 11%+                        | Separate measured CI wave.               |
| Post-8.0   | Dedicated `invalid_payload` integration test      | Lower incidence path; unit coverage on stats. |
| Post-8.0   | Per-model embedding tables / SQLite semantic      | Search-scope work.                       |

## Surprises

- Delta-sync on `/metrics` scrape (same pattern as listener error gauges) avoided
  pulling `metrics` into `maidan-bus`.
- `pg_notify` with a bogus `log_id` was enough to prove `not_found` without mocks.

## Decisions

- **Counters, not gauges** — `maidan_bus_notify_hydrate_total` uses Prometheus counters
  with fixed `result` labels; cumulative atomics sync on scrape.
- **Postgres only** — `InMemoryBus` has no hydrate path; `bus_hydrate_stats` is `None` on SQLite.

## Capability table extension

| Capability                                              | First available in |
|---------------------------------------------------------|--------------------|
| `maidan_bus_notify_hydrate_total` on `/metrics`         | `v8.0.0`           |
| Bus hydrate alerting runbook                            | `v8.0.0`           |

## Risks identified + mitigated

- **Invisible hydrate failures after pointer delivery** — counters + runbooks.
- **Operator confusion on dropped NOTIFY** — docs tie spikes to publish order and replay paths.

## Risks identified + still open

- **At-most-once delivery** — unchanged; replay remains recovery path.
- **Coverage depth** — floor still 10.0%.
- **SQLite semantic search** — still unsupported.

## Forward look

Next wave is open: coverage uplift toward 11%+, or outbox / stronger delivery semantics.
See [[Open Work]].

## Acknowledgements

Solo cluster. Kickoff #164, implementation #165, this retro.

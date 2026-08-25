# Cluster 272.0 retro — search replica-reads metric (deferrals sweep closes)

> Tag **`v272.0.0`**. Phase XXIV (post-gate hardening). **Optional deferrals sweep,
> part 6 (final) — search read-routing observability.** No new gate tag.

## What shipped

- `maidan_search_replica_reads_total{outcome="primary"|"replica"}` — the search-side
  twin of the store's `maidan_replica_reads_total`, so an operator can see the
  primary-vs-replica split for message search independently of store reads.
- `PostgresSearch` gains a metrics-agnostic `SearchReadMetrics` (two atomics + a
  `snapshot()`), incremented in `read_pool()` on the routing decision — but only when
  a replica is configured, so single-primary deployments leave it at zero and the
  metric isn't emitted. `read_routing_metrics()` exposes the handle.
- `main.rs` captures the handle from the concrete `PostgresSearch` (before wrapping it
  in `Arc<dyn Search>`) when `MAIDAN_DB_REPLICA_URL` is set, and stashes it on
  `AppState.search_read_routing_metrics`; `metrics.rs` delta-syncs it into the counter
  each tick (the `LAST_SEARCH_READ_ROUTING` high-water pattern, same as the store's).
- The `#[ignore]`d `replica_routing` search test now also asserts both counters advance
  deterministically (forced-primary via an `Lsn::MAX` token, forced-replica via no
  token) — validated against real streaming replication (`scripts/replica-harness.sh`).

## Surprises / decisions

- **No lag gauge on the search side.** The store's poller already emits
  `maidan_replica_lag_bytes` for the same physical replica search reads from, so
  duplicating it here would be redundant (and could disagree by a poll interval).
  `SearchReadMetrics` is deliberately just the two read counters.
- **Capture the handle before `Arc<dyn Search>`.** `read_routing_metrics()` lives on
  the concrete `PostgresSearch`, not the `Search` trait, so `main.rs` binds
  `pg_search` first, pulls the handle, then `Arc::new(pg_search)` — the exact shape the
  store already uses for `pg_store`.
- **Metrics-agnostic store/search, Prometheus-aware server.** Both crates expose plain
  atomics; the server owns the metric names and the delta-to-counter translation. This
  keeps `maidan-search` free of a metrics-backend dependency (the `HydrateStats` /
  `ReadRoutingMetrics` pattern).

## Capability table extension

| Change | Where |
|--------|-------|
| `maidan_search_replica_reads_total{outcome}` metric | `maidan-server/src/metrics.rs` |
| `SearchReadMetrics` + counted `read_pool` + `read_routing_metrics()` | `maidan-search/src/postgres.rs`, `lib.rs` |
| `AppState.search_read_routing_metrics` + boot capture | `maidan-server/src/state.rs`, `main.rs` |
| counter assertions in the real-replica test | `maidan-search/tests/replica_routing.rs` |
| docs | `docs/Production.md` |

## Risks identified + still open

- None new — an additive, replica-only counter over the Cluster-271 routing.

## Forward look

**This closes the optional-deferrals sweep (Clusters 267–272)** — A2A egress
`content→parts` (267), MCP email tools (268), workspace import store + REST (269–270),
and search token-aware read routing + its metric (271–272). **It also closes the LSN
read-replica program end-to-end**: store reads (262–266) and now search reads (271–272)
both honor the `Maidan-Consistency-Token`, validated against real streaming
replication, with primary/replica counters and a lag gauge. No optional deferrals
remain from the security-led four-program run.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues [[Retros/Cluster 271.0]].

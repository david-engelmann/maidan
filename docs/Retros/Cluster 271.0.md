# Cluster 271.0 retro — search honors the consistency token

> Tag **`v271.0.0`**. Phase XXIV (post-gate hardening). **Optional deferrals sweep,
> part 5 — search token-aware read routing.** No new gate tag.

## What shipped

- `maidan-search`'s `PostgresSearch` now routes its reads to a read replica once the
  replica has caught up to the request's `Maidan-Consistency-Token`, exactly like the
  store (Clusters 262–266). It gains a `reader` pool, a `has_replica` flag, a cached
  replica replay LSN (`Arc<AtomicU64>`) refreshed by a 200 ms background poller, and a
  `read_pool()` selector. `new(pool)` aliases `reader = pool` / `has_replica = false`,
  so single-primary deployments and tests are byte-unchanged; `with_replica_reader`
  wires a real replica.
- **The routing decision is single-sourced.** maidan-store exposes a new
  `pub fn postgres::replica_route(has_replica, cached_replay) -> bool` that reads the
  same `READ_CONSISTENCY` task-local the store's `read_pool` uses and applies the same
  `route_decision`. Search calls it, so there is exactly one place that decides
  primary-vs-replica for a given request token. (`read_pool` was refactored onto a
  shared `route_now`, no behaviour change.)
- Search **reads** route (lexical `search_messages`; semantic `semantic_search` —
  its model-table lookup and the query run against the *same* chosen pool so they can
  never disagree about what is replicated). Search **writes** stay on the primary:
  embedding upserts, `ensure_model` DDL, and reindex.
- `main.rs` builds search its own replica reader pool (separate from the store's) via
  the shared `make_pg_opts` closure when `MAIDAN_DB_REPLICA_URL` is set.
- Validated against **real streaming replication** (`scripts/replica-harness.sh`): the
  `#[ignore]`d `replica_routing` test posts a message, reads-your-write via the token
  (routed to the primary while the replica is behind), waits for the standby to replay
  past the token, then a no-token search is served from the replica — all green
  against an actual primary+standby pair.

## Surprises / decisions

- **Cross-crate task-local sharing is the whole trick.** The consistency middleware
  sets maidan-store's `READ_CONSISTENCY` task-local around GET/HEAD handling; search
  reads are awaited inline in that same task, so exposing one bool helper from
  maidan-store lets search honor the token without its own middleware or a duplicate
  task-local. No cycle: search → store is a normal dep; store → search is dev-only.
- **Semantic search routes as a unit.** Routing the model-table `resolve` to one pool
  and the vector query to another could pick a replica that hasn't replicated a
  brand-new embedding table (→ "relation does not exist"). Computing `read_pool()`
  once and using it for the lookup, the optional `SET LOCAL ef_search` tx, and the
  query keeps them consistent.
- **No new search-specific metric here.** The store's `maidan_replica_reads_total` +
  `maidan_replica_lag_bytes` already observe the same replica search reads from, so a
  parallel `maidan_search_replica_reads_total` is deferred to 272 rather than dragging
  the `ReadRoutingMetrics` plumbing (AppState + metrics.rs delta-sync) into this
  cluster.

## Capability table extension

| Change | Where |
|--------|-------|
| `PostgresSearch` reader pool + replay poller + `read_pool()` routing | `maidan-search/src/postgres.rs` |
| `postgres::replica_route` (shared routing bool) + `route_now` refactor | `maidan-store/src/postgres/mod.rs` |
| search replica reader wired at boot | `maidan-server/src/main.rs` |
| real-replica validation (`#[ignore]`d) | `maidan-search/tests/replica_routing.rs` |
| docs: search routes too | `docs/Production.md` |

## Risks identified + still open

- **Search offload has no dedicated counter yet** — 272 adds
  `maidan_search_replica_reads_total{outcome}` for the primary/replica split.
- A brand-new embedding model's table, created on the primary within the token
  window, is only searchable from the replica once replicated — the token routing
  already handles this (a token-scoped search stays on the primary until caught up); a
  no-token search may briefly miss it. Acceptable (no-token = "no causality need").

## Forward look

Cluster 272 closes observability parity — `maidan_search_replica_reads_total` — and
with it the entire optional-deferrals sweep (and the LSN read-replica program).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues [[Retros/Cluster 270.0]].

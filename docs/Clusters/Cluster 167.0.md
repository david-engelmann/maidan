# Cluster 167.0 — perf: rate-limiter eviction + embedding model cache

**Theme:** Arc 2 (perf), part 2 — a memory leak and the embedding-upsert
round-trip cost.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v167.0.0`**, no new gate tag.

---

## Scope

| Fix | Where |
|-----|-------|
| Bound the in-memory rate-limiter map (evict elapsed windows) — R2 | `rate_limit/limiter.rs` |
| Cache `model → table_name` on `PostgresSearch` — H6 | `search/postgres.rs` |

## Why

- **R2:** `MEMORY_BUCKETS` entries were reset in place but never removed, so the
  map grew unbounded as distinct keys accumulated — a memory leak on the
  no-Redis rate-limit path. It now sweeps entries whose window has fully elapsed
  once the map crosses `MEMORY_SWEEP_THRESHOLD`.
- **H6:** `upsert_embedding` called `ensure_model_postgres` on every call — a
  `maidan_embedding_models` SELECT (+ `CREATE TABLE IF NOT EXISTS` checks) even in
  the steady state where the model is already registered. Caching `model →
  table_name` skips those, roughly halving the round-trips in the live indexer +
  reindex hot path. The cache lock is dropped before the `await`, so it's never
  held across it.

## Non-goals

- H4 (outbox JOIN), H2 (delivery-cursor coalesce), R1 (`BROADCAST_CAP`) — next
  cluster. CI/CD workflow speedups — deferred until GitHub Actions recovers.

## Exit criteria

- Rate-limiter map is bounded; embedding upserts hit the cache after first
  resolution; suites green — **met**.
- `v167.0.0` tagged.

## Verification & limits

- `memory_map_evicts_expired_windows_when_large` (fills past the threshold with
  1 ns windows → the sweep shrinks the map). Search lib + rate-limit suites
  green; the model cache is exercised by the existing semantic-search
  testcontainer upsert tests.
- Limit: the cache assumes a model's table never changes once registered (true);
  a dimension mismatch on a cached model surfaces at INSERT (pgvector) rather
  than as the typed `DimensionMismatch`.
- **CI note:** GitHub Actions outage — validated locally; re-run CI on `main`.

## References

- [[Retros/Cluster 167.0]]; `rate_limit/limiter.rs`, `search/postgres.rs`.
  Program: [[Roadmap]] + memory `maidan-next-arc-program`.

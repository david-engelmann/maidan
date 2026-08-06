# Cluster 167.0 retro — rate-limiter eviction + embedding model cache

> Tag **`v167.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Arc 2 (perf), part 2.

## What shipped

- **R2:** the in-memory rate-limiter bucket map now sweeps fully-elapsed windows
  once it crosses `MEMORY_SWEEP_THRESHOLD`, bounding what was an unbounded map
  (memory leak). `MemoryCounter` gained its `window` so a sweep can tell which
  entries are stale.
- **H6:** `PostgresSearch` caches `model → table_name`; steady-state embedding
  upserts skip the `ensure_model_postgres` SELECT + create-checks.

## What was deferred / not covered

| Item | Why |
|------|-----|
| H4 / H2 / R1 | Next perf cluster. |
| CI/CD workflow speedups | Blocked by the GitHub Actions outage. |

## Surprises

- **The map couldn't self-describe staleness.** Eviction needs to know each
  entry's window, but `MemoryCounter` only stored `window_start` + `count`, and
  the shared map mixes keys from *different* limiter configs (per-client vs
  per-workspace, different windows). Storing the `window` per entry was the small
  change that made a correct sweep possible.

## Decisions

- **Sweep on threshold, not on a timer.** No background task — the sweep piggy-
  backs on `try_acquire` when the map is already large, so it costs nothing until
  there's actually something to reclaim.
- **Drop the cache lock before the await.** A `std::sync::Mutex` guard must not
  cross an `.await`; the cache read is scoped to a block that ends before
  `ensure_model_postgres` runs, and the write re-locks after.

## Capability table extension

| Fix | Where |
|-----|-------|
| Bounded rate-limiter map; embedding model→table cache | `rate_limit/limiter.rs`, `search/postgres.rs` |

## Risks identified + still open

- **Low.** R2 only drops already-elapsed windows (no effect on live limits); H6
  is transparent (same table, fewer round-trips). Shipped during the GitHub
  Actions outage; re-run CI on `main` when recovered.

## Forward look

Arc 2 perf finishes with H4 (outbox `list_pending` JOINs the payload + batches
`mark_published`), H2 (coalesce per-subscriber delivery-cursor writes), and R1
(env-tunable `BROADCAST_CAP`) — then the CI/CD workflow speedups once GitHub
Actions is back.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].

# Cluster 199.0 — perf: concurrent workspace-context assembly

**Theme:** Arc D (performance & scale), part 2 — the first optimization on top of
the 198 harness: stop building a workspace-context pack's threads one at a time.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v199.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| `build_workspace_context` builds page threads via a bounded `buffered` stream (`CONTEXT_THREAD_CONCURRENCY = 8`) instead of a sequential `for` loop | `crates/maidan-server/src/thread_context.rs` |
| No-cross-contamination + page-order correctness test | `crates/maidan-server/tests/workspace_context_concurrency_e2e.rs` |

## Why

`build_thread_context` already batches its *internal* reads (references, edits —
earlier clusters killed the per-message N+1). But `build_workspace_context`
assembled a page of up to 50 threads by calling `build_thread_context` in a
sequential `for` loop, and each such build is ~7 independent store round-trips.
So a workspace-context pack's latency was `Σ (per-thread round-trips)` — it grew
linearly with the number of threads on the page, even though the per-thread
builds are completely independent of one another.

## The change

Replace the sequential loop with a bounded-concurrency stream:

```rust
let threads: Vec<ThreadContext> = stream::iter(thread_ids)
    .map(|tid| build_thread_context(store, tid, limits))
    .buffered(CONTEXT_THREAD_CONCURRENCY)   // = 8
    .try_collect()
    .await?;
```

`buffered` runs up to 8 per-thread builds at once and **yields results in input
order**, so the page ordering contract (and the keyset pagination cursor built
from it) is unchanged. `try_collect` short-circuits on the first error, so the
tombstoned-mid-build 404 behaviour is preserved. The concurrency cap keeps a
single request from opening ~350 concurrent queries (50 threads × 7) and
saturating the connection pool — it collapses wall-clock toward
`ceil(N/8) × single-build` rather than firing an unbounded fan-out.

## Exit criteria

- A workspace-context page builds its threads concurrently, order + query-count +
  error semantics unchanged — **met**.
- `v199.0.0` tagged.

## Verification & limits

- `context_query_count_e2e` stays green — the **query count is identical**; only
  the concurrency changed (this is a reshuffle of when queries run, not a
  reduction).
- New `workspace_context_concurrency_e2e`: 12 threads each with a message tied to
  its own id; the pack returns all 12, each context carries *its own* message (no
  cross-thread mixup under concurrency), in page order.
- Measure with the 198 harness (a workspace-context op on a many-thread
  workspace) to see the wall-clock drop; the win scales with threads-per-page.
- Limit: the route still **builds every page thread, then RBAC-filters** — work
  is spent assembling contexts for threads the caller can't see. Filter-before-
  build is a larger refactor (the builder would need the auth context) — logged
  in Open Work. Concurrency cap (8) is a fixed constant, not tuned per backend.

## References

- [[Retros/Cluster 199.0]]; `crates/maidan-server/src/thread_context.rs`.
  Program: [[Roadmap]] + memory `maidan-next-arc-program` (Arc D). Builds on the
  [[Retros/Cluster 198.0]] harness.

# Cluster 199.0 retro — a context pack's threads build in parallel now

> Tag **`v199.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Arc D (performance & scale), part 2.

## What shipped

- `build_workspace_context` builds each page thread's context concurrently via a
  bounded `buffered` stream (cap 8) instead of a sequential `for` loop —
  collapsing a many-thread pack's latency from `Σ per-thread` toward
  `ceil(N/8) ×` a single build, order + query-count + error semantics unchanged.

## Surprises / decisions

- **The N+1 was one level up from where prior clusters looked.**
  `build_thread_context` was already carefully batched internally (references and
  edits are single reads across all messages — earlier clusters). But the
  *workspace* pack called it in a plain `for` loop, so the win wasn't fewer
  queries per thread, it was *not waiting for each thread's queries to finish
  before starting the next thread's*. Independent work run sequentially.
- **`buffered`, not `buffer_unordered` or `try_join_all`.** Three options: an
  unbounded `try_join_all` (fires all 50 threads × 7 queries ≈ 350 concurrent
  acquires at the pool — a self-inflicted thundering herd), `buffer_unordered`
  (fastest but drops the page-order contract that the pagination cursor depends
  on), or `buffered` (bounded + order-preserving). The pagination cursor and the
  response contract make order non-negotiable, and pool safety makes the bound
  non-negotiable — `buffered` is the only one that satisfies both.
- **Query count is the tell that this is safe.** `context_query_count_e2e` still
  passes unchanged: the *same* queries run, just overlapped. That's the proof the
  change is a pure scheduling reshuffle, not a semantic edit — no risk of a
  batched query returning a different set.
- **Concurrency is where cross-contamination bugs hide.** Parallelizing per-entity
  assembly is exactly where a shared-buffer or wrong-id bug would surface as
  "thread A's context has thread B's messages". Each `build_thread_context` owns
  its own locals, so it's safe — but I added a test that seeds each thread with a
  message tied to its own id and asserts every returned context carries *its own*
  message. That's the assertion that would catch a future refactor that broke
  isolation.

## Decisions

- **Cap = 8.** Enough to overlap the common page sizes without a single request
  monopolizing the pool; a fixed constant rather than a tuned/per-backend knob
  (premature until the harness says otherwise).
- **Preserve the tombstoned-mid-build 404.** `try_collect` short-circuits on the
  first `Err`, exactly like the old loop's `?`. A perf change shouldn't quietly
  turn a 404 into a partial 200.

## Capability table extension

| Change | Where |
|--------|-------|
| Concurrent (bounded) per-thread build in the workspace-context pack | `crates/maidan-server/src/thread_context.rs` |

## Risks identified + still open

- **Build-then-filter waste.** The route builds every page thread's context, then
  drops the ones the caller can't access. So this cluster made the *wasted* work
  concurrent too — the right long-term fix is filter-before-build (the builder
  would take the auth context), a bigger refactor logged in Open Work.
- **Fixed cap.** 8 is a guess, not a measurement; the 198 harness is the tool to
  tune it if a real workload wants more.

## Forward look

Arc D continues: workspace-sharded fan-out + shared reconcile, filtered-ANN
search, batched `pg_notify`, read-replica routing. Baseline each with `loadgen`
before, re-run after.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Builds on the
[[Retros/Cluster 198.0]] load harness.

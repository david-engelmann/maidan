# Cluster 281.0 retro — a published, reproducible benchmark

> Tag **`v281.0.0`**. Phase XXIV (post-gate hardening). **Launch-readiness P1:
> published benchmark methodology.** No new gate tag.

## What shipped

Evidence for the performance story, produced by running the harness, not by
asserting an adjective:

- **`post_to_observer_latency`** — a new `#[ignore]`d measurement in the Cluster-198
  loadgen harness: time from a producer initiating a message post to a subscribed
  WebSocket observer receiving it. Reads the matching event *concurrently* with the
  POST (via `tokio::join!`) so the sample is fan-out, not the POST round-trip.
  Correlates by a per-op nonce in the body; authenticates the `/ws/subscribe` upgrade
  via the subscribe frame's `token` field (not an HTTP header). A pure `http_to_ws`
  helper is unit-tested in CI alongside the existing percentile math.
- **`docs/Benchmark.md`** (published in the book) — named hardware / commit / OS /
  toolchain / backend, the two measurements, reproduction commands, and honest
  caveats. Linked from the README.

## Numbers (Apple M3 Max, SQLite in-process, one connection, `v280.0.0`)

- **Post→observer:** p50 **0.71 ms**, p99 1.00 ms (200 samples, 0 errors).
- **Throughput (mixed post/read/search):** 8 workers → **1 586 ops/s**, post p50 6.2
  ms; 32 workers → 666 ops/s, post p50 49 ms. Zero errors at both.

## Surprises / decisions

- **The harness was benchmarking a configuration Maidan does not ship.**
  `spawn_in_process` opened SQLite with **16 connections**, but Cluster 277 ships a
  **one-connection** default precisely because a multi-connection SQLite pool
  deadlocks under write contention. The first run bore this out: 8 and 23 spurious
  errors — the very deadlock 277 fixed. Switched the harness to
  `DEFAULT_SQLITE_MAX_CONNECTIONS` (with `min_connections(1)` to keep the in-memory
  DB alive) → **0 errors**, and the numbers now describe the real product.
- **The single-writer ceiling is a feature to report, not hide.** Throughput is
  higher at 8 workers than at 32 because every query serializes through the one SQLite
  connection. The honest framing — SQLite is the local/edge backend with a
  single-writer ceiling; Postgres is the production multi-writer backend — is exactly
  the positioning, now with data. Correctness holds throughout (0 errors); contention
  shows up as latency, not failure.
- **Measured, not asserted.** The README carried no unbacked "high-performance"
  adjective to walk back (the pitch already says "spends only the tokens it needs"),
  so the benchmark is purely additive evidence.
- **Postgres one-command target deferred.** The harness benchmarks Postgres today via
  `MAIDAN_LOADGEN_URL` against a running deployment; a first-class in-harness Postgres
  testcontainer target (so multi-writer numbers sit beside the SQLite ones) is a
  logged follow-up rather than scope creep here.

## Capability table extension

| Change | Where |
|--------|-------|
| Post→observer realtime-propagation latency measurement | `crates/maidan-server/tests/loadgen.rs` |
| Loadgen SQLite target uses the shipped 1-connection default (was 16 → deadlocked) | `crates/maidan-server/tests/loadgen.rs` |
| Published benchmark: named config, numbers, reproduction, caveats | `docs/Benchmark.md`, `book/src/SUMMARY.md`, `book/sync-docs.sh`, `README.md` |

## Risks identified + still open

- **One machine, in-process.** Absolute numbers are a reference floor, not an SLA;
  the doc says so and gives the method to reproduce on real hardware/network.
- **No in-harness Postgres number yet.** The production multi-writer story is
  described and reproducible via an external URL, but not one-command. Logged.

## Forward look

Last of the launch-readiness backlog: **A2A v1.0 compliance** — a multi-cluster arc
seeded by the review's JSON-RPC gap matrix. Also the deferred framework interop CI job
and the in-harness Postgres benchmark target.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues
[[Retros/Cluster 280.0]]. Benchmark ask from the external launch-readiness review
(Cluster 274); numbers produced here by running the harness.

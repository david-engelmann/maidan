# Benchmark

Maidan ships a reproducible load/latency harness, and this page reports the
numbers it produced on named hardware and a named commit. These are a **reference
baseline you can reproduce**, not a marketing SLA: they were measured against the
single-node SQLite backend on one machine, in-process (no network hop). Treat them
as a floor and a methodology, then re-run against your own deployment.

## What is measured

Two things, from the harness in
[`crates/maidan-server/tests/loadgen.rs`](https://github.com/david-engelmann/maidan/blob/main/crates/maidan-server/tests/loadgen.rs)
(introduced in Cluster 198, extended with the post→observer measurement in Cluster
281):

1. **Throughput + REST latency** (`load_baseline`) — concurrent workers each loop
   over post-a-message → read-the-thread → search-the-workspace, reporting
   per-operation latency percentiles (nearest-rank) and overall operations/second.
2. **Post→observer latency** (`post_to_observer_latency`) — the realtime
   propagation number: the time from a producer initiating a message post to a
   subscribed observer receiving that message over the WebSocket (`/ws/subscribe`).
   Each iteration reads the matching event *concurrently* with the POST, so the
   sample reflects fan-out, not the POST round-trip.

Both are `#[ignore]`d — they are measurement tools, not pass/fail CI gates (a hard
latency floor would flake across runner hardware). The percentile math itself is
pure and unit-tested in CI.

## Configuration (this run)

| Axis | Value |
|------|-------|
| Date | 2026-08-26 |
| Commit | Cluster 281 harness on top of `v280.0.0` (`7abf67a`); the 281 change is test-only, so the measured server is `v280.0.0`) |
| Hardware | Apple M3 Max, 16 cores, 128 GB RAM |
| OS | macOS 14.6 (arm64) |
| Toolchain | rustc 1.91.1, `--release` |
| Backend | SQLite (`sqlite::memory:`), **one connection** (the shipped default, Cluster 277), auth enabled |
| Embeddings | `hash-v1` (offline default; a real provider adds its own network latency) |
| Transport | in-process over loopback (no network hop) |

## Results

### Throughput + REST latency (`load_baseline`)

Each iteration is one post + one read + one search. Latencies are milliseconds.

| Concurrency | Ops | Errors | Wall | Throughput | post p50 / p95 / p99 | read p50 | search p50 |
|-------------|-----|--------|------|------------|----------------------|----------|------------|
| 8 workers × 50   | 1 200 | 0 | 0.76 s | **1 586 ops/s** | 6.2 / 8.3 / 9.5 | 4.4 | 4.2 |
| 32 workers × 100 | 9 600 | 0 | 14.41 s | **666 ops/s** | 49.0 / 124.0 / 151.0 | 34.3 | 39.4 |

### Post→observer latency (`post_to_observer_latency`)

200 serial samples, one message in flight at a time. Milliseconds.

| p50 | p95 | p99 | max | errors |
|-----|-----|-----|-----|--------|
| **0.71** | 0.91 | 1.00 | 1.14 | 0 |

### Context-pack token savings (`token_pack`)

The README says agents "pull exactly the context a step needs … instead of re-stuffing
the prompt, so the same work costs far fewer tokens." Measured: a scoped thread **context
pack** (`GET /threads/:id/context` — a bounded window + pins + results + references +
artifacts, edits as metadata) vs the naive baseline of dumping **every message in the
channel** into the prompt. Reported in **bytes** (exact — the serialized JSON is what an
agent receives) and an estimated token count (`≈ chars/4`; the *ratio* is
tokenizer-independent). Fixture: 8 threads × 40 substantive messages = 320 total; the
target thread also carries 15 edits.

| What the agent is handed | Bytes | ~Tokens |
|--------------------------|-------|---------|
| Scoped pack (`GET …/context`) | 19 802 | ~4 951 |
| Naive: dump the whole channel | 135 630 | ~33 908 |
| Same pack, but full edit bodies | 25 943 | ~6 486 |

- **Scoped pack vs naive channel dump: ~6.8× fewer tokens.**
- **Lean edits (metadata) vs full `body_before`/`body_after`: ~1.3× fewer tokens on the pack** — the single biggest per-pack lever (default `include_edits=false`).

The ratio grows with channel size (the pack is bounded; the dump is not) and with edit
history. This is a data-shape measurement, not a latency one, so it is hardware- and
tokenizer-independent to first order.

## How to read these

- **Realtime fan-out is sub-millisecond.** A subscriber sees a posted message in
  ~0.7 ms p50 / ~1 ms p99 in-process. Add your network round-trip for a wire number.
- **SQLite has a single-writer ceiling, and that is by design.** Throughput is
  higher and latency lower at 8 concurrent workers (1 586 ops/s, post p50 6 ms) than
  at 32 (666 ops/s, post p50 49 ms): every query serializes through the one
  connection Maidan uses for SQLite (Cluster 277 chose one connection because a
  multi-connection SQLite pool deadlocks under write contention). Note there are
  **zero errors** at both levels — correctness holds; the cost of contention shows
  up as latency, not failures. SQLite is the local-dev / edge backend. For
  production write concurrency, use Postgres, whose multi-writer story removes this
  ceiling.
- **These are one machine, in-process.** Absolute numbers will differ on your
  hardware and under a real network. The value here is the method and the shape.

## Reproduce it

```sh
# throughput + REST latency (default 8 workers × 50 iterations):
cargo test --release -p maidan-server --test loadgen load_baseline -- --ignored --nocapture

# tune concurrency / switch to a timed soak:
MAIDAN_LOADGEN_CONCURRENCY=32 MAIDAN_LOADGEN_OPS=100 \
  cargo test --release -p maidan-server --test loadgen load_baseline -- --ignored --nocapture

# post→observer latency (default 200 samples):
cargo test --release -p maidan-server --test loadgen post_to_observer_latency -- --ignored --nocapture

# both, wrapped with the env knobs:
scripts/loadgen.sh

# context-pack token savings (bytes + estimated tokens + the ratio):
cargo test -p maidan-server --test token_pack -- --ignored --nocapture
```

The `token_pack` estimator math is pure and unit-tested in CI; the measurement itself is
`#[ignore]`d (a measurement tool, not a gate), like `load_baseline`.

Env knobs: `MAIDAN_LOADGEN_CONCURRENCY`, `MAIDAN_LOADGEN_OPS`,
`MAIDAN_LOADGEN_DURATION_SECS` (timed soak), `MAIDAN_LOADGEN_OBSERVER_OPS`.

### Against an external / Postgres deployment

The harness targets a running server (any backend) when you give it a base URL,
a bearer token, and the ids to drive:

```sh
MAIDAN_LOADGEN_URL=http://localhost:8080 \
  MAIDAN_LOADGEN_BEARER=maid_... \
  MAIDAN_LOADGEN_IDS='<workspace>|<channel>|<thread>|<member>' \
  cargo test --release -p maidan-server --test loadgen -- --ignored --nocapture
```

This is how to benchmark a Postgres-backed deployment today. A first-class,
one-command Postgres benchmark target (spinning up a Postgres testcontainer inside
the harness so the multi-writer numbers sit next to the SQLite ones) is tracked as a
follow-up in [Open Work](Open%20Work.md).

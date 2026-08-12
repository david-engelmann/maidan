# Cluster 198.0 — perf: load / soak harness (Arc D opener)

**Theme:** Arc D (performance & scale), part 1 — the baseline. Before optimizing
anything, build a repeatable way to drive concurrent traffic at the server and
report latency percentiles + throughput.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v198.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| `load_baseline` — `#[ignore]`d concurrent load driver (post/read/search × N workers), reports per-op percentiles + throughput | `crates/maidan-server/tests/loadgen.rs` |
| `stats` — pure nearest-rank percentile function + unit tests (runs in CI) | same |
| `loadgen.sh` — env-knob runner (`--release … --ignored --nocapture`) | `scripts/loadgen.sh` |
| "Load & soak testing" section | `docs/Operations.md` |

## Why

Arc D is a sequence of performance optimizations — workspace-sharded fan-out,
filtered-ANN search, batched `pg_notify`, read-replica routing, batched context
assembly. Each is a claim ("this makes X faster") that is worthless without a way
to measure X. The existing tooling doesn't cover this: the Criterion benches in
`maidan-store`/`maidan-search` are single-threaded micro-benchmarks, and
`scale-out-smoke.sh` proves multi-replica *functionality*, not throughput under
concurrent load. So Arc D opens by building the measuring stick.

## The design

`load_baseline` spins `MAIDAN_LOADGEN_CONCURRENCY` worker tasks (a
`tokio::JoinSet`), each looping a fixed op-mix — post a message, read the thread,
search the workspace — either for `MAIDAN_LOADGEN_OPS` iterations or (soak) until
`MAIDAN_LOADGEN_DURATION_SECS` elapses. Each op's latency is sampled only on a
2xx; failures increment an error counter. The workers' samples are merged and
`stats` computes nearest-rank min/mean/p50/p95/p99/max per op kind, printed with
overall ops/s.

Two decisions keep it honest and low-cost:

- **`#[ignore]`d, not a CI gate.** A hard latency assertion flakes across runner
  hardware and CI contention. The load run is a *tool* you invoke; the only thing
  that runs in CI is the pure percentile math (unit-tested — `1..=100` →
  p50=50/p95=95/p99=99, empty → none, single → every-percentile).
- **In-process by default, external by env.** No `MAIDAN_LOADGEN_URL` → spin up an
  in-process SQLite server (auth enabled, seeded ws/channel/thread/token) and hit
  `127.0.0.1`. Set the URL (+ `_BEARER` + `_IDS=ws|ch|thread|member`) to point the
  same driver at a live/scaled Docker deployment.

Low-ripple: a `tests/` file + a script — no new crate or bin, so no
bootstrap-strip (`--no-default-features`) or `cargo-deny` fallout, and unwraps are
fine in test code.

## Exit criteria

- A single command drives concurrent load and prints per-op latency percentiles +
  throughput; the percentile math is unit-tested — **met**.
- `v198.0.0` tagged.

## Verification & limits

- `stats_tests` (run in CI): nearest-rank correctness, empty, single-sample.
- Manual run (`MAIDAN_LOADGEN_CONCURRENCY=6 MAIDAN_LOADGEN_OPS=20`): 360 ok ops, 0
  errors, ~1.8k ops/s, sub-10ms p99s on the in-process SQLite path.
- Limit: the in-process SQLite path measures **app logic**, not the deployed
  network/Postgres/object-store stack — for the real thing, point `_URL` at a
  compose/scale deployment. The op-mix is fixed (post/read/search); adding WS/MCP
  workloads or a weighted mix is a future extension. No historical baseline store
  yet (capture reports by hand per optimization).

## References

- [[Retros/Cluster 198.0]]; `crates/maidan-server/tests/loadgen.rs`,
  `scripts/loadgen.sh`. Program: [[Roadmap]] + memory `maidan-next-arc-program`
  (Arc D). Baseline for the Arc D optimizations that follow.

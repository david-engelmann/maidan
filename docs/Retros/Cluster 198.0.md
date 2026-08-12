# Cluster 198.0 retro — measure before you optimize (Arc D opens)

> Tag **`v198.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Arc D (performance & scale), part 1 — the baseline.

## What shipped

- A load / soak harness: `scripts/loadgen.sh` + the `#[ignore]`d `load_baseline`
  test drive concurrent REST traffic (post/read/search) and report per-op latency
  percentiles + throughput. The pure percentile math is unit-tested in CI.

## Surprises / decisions

- **Arc D opens with a measuring stick, on purpose.** Every remaining Arc D
  cluster is a performance claim. Shipping the harness *first* means each of those
  can be shown to move a number instead of asserted to. It's the least glamorous
  cluster of the arc and the one that makes the rest honest.
- **The load run must not be a CI gate.** The instinct is to assert "p99 < N ms"
  so a regression fails the build. But CI runner hardware and contention vary
  wildly — a latency floor flakes, gets bumped up to stop flaking, and then
  catches nothing. So the load run is `#[ignore]`d (a tool you invoke), and the
  *only* thing wired into CI is the pure percentile function's unit tests. That
  split keeps a real, deterministic check without a flaky one.
- **In-process by default, external by env — one driver, two targets.** The same
  worker loop hits an in-process SQLite server (zero setup, measures app logic) or
  a live Docker/scale deployment (`MAIDAN_LOADGEN_URL` + `_BEARER` + `_IDS`). The
  in-process path is what you reach for iterating on an optimization; the external
  path is for validating it against the real stack.
- **Low-ripple placement mattered.** A load-gen *binary* (new crate or `src/bin`)
  would drag in bootstrap-strip's `--no-default-features` build, `cargo-deny`, and
  the strict `-D clippy::unwrap_used` lint (no unwraps in a bin) — a lot of
  ceremony for a dev tool. Putting it in `tests/` sidesteps all of that: unwraps
  are fine, it compiles with the existing dev-deps (reqwest/tokio), and `#[ignore]`
  keeps it out of the default run.

## Decisions

- **Nearest-rank percentiles**, not interpolation — simpler, and for latency
  triage the exact discretization doesn't matter. Unit-tested against `1..=100`.
- **Fixed op-mix (post/read/search)** as the v1 workload — the three hottest REST
  paths. A weighted/configurable mix and WS/MCP workloads are future extensions,
  not needed to baseline the first optimizations.
- **Sample only 2xx**; count failures separately. A latency percentile
  contaminated by fast error responses would lie.

## Capability table extension

| Change | Where |
|--------|-------|
| Load / soak harness (`loadgen.sh` + `load_baseline`) — concurrent REST load → latency percentiles + throughput | `crates/maidan-server/tests/loadgen.rs`, `scripts/loadgen.sh` |

## Risks identified + still open

- **The default target is in-process SQLite** — it measures app logic, not the
  deployed Postgres/network/object-store stack. That's the right default for
  iterating on an optimization, but a real capacity number needs the external
  target against a compose/scale deployment. Open: no historical baseline store
  (capture reports by hand); fixed op-mix; REST only (no WS/MCP load yet).

## Forward look

Arc D continues, now measurable: workspace-sharded fan-out + shared reconcile,
filtered-ANN search, batched `pg_notify`, read-replica routing, batched context
assembly. Capture a `loadgen` baseline before each and re-run after.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Opens Arc D
(performance & scale).

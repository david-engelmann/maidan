# Cluster 9.0 retro — Coverage depth

> Closing wave for Cluster 9.0 · target tag `v9.0.0`.

Cluster 9.0 raised the CI line-coverage floor with targeted tests in recently
touched crates, without changing runtime behavior.

## What shipped

- **PR #167** — Implementation bundle (kickoff plan + 9.0.1–9.0.3):
  - Tests: `EventFilter` matching, `BusError` display, `HydrateStats` labels,
    `subscribe_metrics` recording, `/metrics` hydrate scrape e2e, search query
    edge case, auth peer decrypt failure.
  - `COVERAGE_MIN_LINES` **10.0 → 10.5** (CI coverage job green).
  - [[Operations]] documents Cluster 9.0 bump policy; `11.0` deferred until CI
    confirms headroom.
  - WS auto-replay e2e timeout extended for slow CI hosts (flaky integration fix).

## What was deferred

| To         | What                                              | Why                                      |
|------------|---------------------------------------------------|------------------------------------------|
| Post-9.0   | `COVERAGE_MIN_LINES` toward **11.0**              | Incremental bump policy; 10.5 green first. |
| Post-9.0   | Outbox / at-least-once delivery                   | Separate reliability epic.               |
| Post-9.0   | Per-model embedding tables / SQLite semantic      | Search-scope work.                       |

## Surprises

- Full-workspace `llvm-cov` locally was impractically slow; CI coverage job was
  the calibration source for `10.5`.
- Integration failure on first run was an unrelated flaky WS lag test, not the new
  coverage tests.

## Decisions

- **Bump-below-measured via CI** — set `10.5` after green coverage job on the PR,
  not `11.0` on first attempt (Cluster 5 lesson).
- **Behavior-focused tests** — no blanket line-padding; tests assert real paths.

## Capability table extension

| Capability                                              | First available in |
|---------------------------------------------------------|--------------------|
| CI line-coverage floor at 10.5%                         | `v9.0.0`           |
| Targeted coverage tests (bus/types/server metrics)      | `v9.0.0`           |

## Risks identified + mitigated

- **Coverage regression** — floor raised with additive tests that exercise 7.0–8.0 paths.
- **Flaky WS lag e2e** — timeout 10s → 20s for CI variance.

## Risks identified + still open

- **At-most-once delivery** — unchanged.
- **Coverage still modest** — 11%+ remains a standing goal.
- **SQLite semantic search** — still unsupported.

## Forward look

Next wave: measured push toward **11.0** coverage floor, or outbox/delivery
semantics design. See [[Open Work]].

## Acknowledgements

Solo cluster. Implementation #167, this retro.

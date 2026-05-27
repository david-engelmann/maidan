# Cluster 9.0 — Coverage depth

Cluster 8.0 closed bus hydrate observability at **`v8.0.0`**. Four prior clusters
(6–8) focused on delivery and bus operations; CI still enforces a **10.0%** line
floor from **`v5.0.0`**, and retros since 5.0 repeatedly deferred a measured bump
toward **11%+** (`COVERAGE_MIN_LINES=11.0` failed on first attempt in Cluster 5).

> **Goal:** Add focused unit and integration tests in high-risk, recently touched
> crates; re-measure line coverage on green `main`; raise `COVERAGE_MIN_LINES` to
> slightly below the new measurement; document the bump in [[Operations]].
>
> **Target tag:** `v9.0.0`.

## PRs

| #         | Title                                                                  | Issue |
|-----------|------------------------------------------------------------------------|-------|
| kickoff   | `docs: Cluster 9.0 kickoff plan`                                       | TBD   |
| 9.0.1     | `test: targeted coverage tests (bus, types, server metrics)`           | TBD   |
| 9.0.2     | `ci: raise COVERAGE_MIN_LINES after re-measurement`                    | TBD   |
| 9.0.3     | `docs: coverage bump policy in Operations`                             | TBD   |
| 9.0.retro | `docs(retro): Cluster 9.0 retrospective + v9.0.0 tag prep`            | TBD   |

## Order

1. **9.0.1** — add tests (no blanket `unwrap` padding):
   - `maidan-types`: `EventFilter::matches` / `matches_envelope` edge paths.
   - `maidan-bus`: `BusError` display; `HydrateStats` all `result` labels.
   - `maidan-server`: `subscribe_metrics` record paths; `/metrics` scrape with
     `bus_hydrate_stats` set (extend `metrics_e2e` or lib test with `metrics::init`).
   - Prioritize crates touched in 7.0–8.0 plus existing low-coverage helpers.
2. **9.0.2** — run `cargo llvm-cov --workspace --lib --bins` on green branch;
   set `COVERAGE_MIN_LINES` in `.github/workflows/ci.yml` **below** measured
   (bump-below-measured policy from Cluster 5; target **11.0** only if CI green).
3. **9.0.3** — [[Operations]]: record CI run id, new floor, re-measure instructions.
4. **9.0.retro** + `v9.0.0` tag.

## Exit criteria

- CI green on `main` (five required checks + raised coverage floor).
- `COVERAGE_MIN_LINES` reflects a fresh measurement documented in [[Operations]].
- [[Retros/README]] includes Cluster 9.0; `v9.0.0` tagged.

## Risks

| Risk | Mitigation |
|------|------------|
| `11.0` gate fails again | Measure first; set floor below green `main` (e.g. 10.5) |
| Flaky coverage on different hosts | CI is source of truth; document run id |
| Low-value tests | Focus on behavior assertions, not line-padding |

## Out of scope

- Outbox / at-least-once delivery semantics.
- Codecov workflow changes (shipped in 5.0).
- Semantic search / model-table work.
- Bus pointer or hydrate behavior changes.

## Alternative next cluster (not this wave)

**Outbox / stronger delivery** — deferred while reliability observability stack
(6–8) is fresh; needs dedicated multi-PR design.

## References

- Cluster 5.0 coverage playbook: [[Clusters/Cluster 5.0]], [[Retros/Cluster 5.0]].
- CI coverage job: `.github/workflows/ci.yml`.
- Recent bus code: `maidan-bus` pointer/hydrate, `maidan-server/src/metrics.rs`.

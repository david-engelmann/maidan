# Cluster 114.0 — Coverage uplift + fuzz

**Theme:** Make the coverage gate meaningful and raise the floor; add fuzz/round-trip coverage to the JSON-RPC / MCP / A2A envelope surface.

**Ladder:** [[Clusters/Product Ladder 102+]] Phase XXI · tag **`v114.0.0`**.

**Predecessor:** Coverage gate from Cluster 11 (`COVERAGE_MIN_LINES`); MCP/A2A protocol crates.

---

## Problem

The `coverage` CI job ran `cargo llvm-cov --workspace --lib --bins` — inline unit tests only — so the gated number (~16%) excluded every `tests/` integration suite. Most store/server logic is exercised by integration tests, not units, so the gate bore little relation to what the suite actually covers and **could not** be raised toward the ladder's 40% target by adding units (no realistic amount of inline testing moves a 23k-line workspace from 16 → 40 when the bulk is DB/HTTP glue). Separately, the JSON-RPC / MCP / A2A envelope types had no inline tests.

## Scope

| Layer | Deliverable |
|-------|-------------|
| **Tests** | Round-trip + `proptest` fuzz tests for the JSON-RPC / MCP / A2A envelope surface (`maidan-mcp` protocol + error, `maidan-a2a` protocol). |
| **CI** | Switch the coverage job to gate on the *whole* test suite (`cargo llvm-cov nextest --workspace` + `docker:dind`), reported once into lcov + html. |
| **Floor** | Ratchet `COVERAGE_MIN_LINES` from 11 toward 40 based on the measured full-suite number (~60%). |

## Non-goals

- Pushing the floor all the way to the ~60% ceiling — 40 is the ladder target; tighter is a later ratchet.
- A dedicated `cargo-fuzz` libFuzzer harness — "fuzz" here is property-based round-trip (`proptest`).
- Changing any `src/` behavior beyond inline test modules.

## PR ladder (actual)

| # | Title |
|---|--------|
| 114.0.1 | `test(mcp,a2a): envelope round-trip + fuzz tests` (#314) |
| 114.0.2 | `ci(coverage): gate on full-suite coverage; ratchet floor 11 -> 40` (#314) |
| 114.0.retro | `docs(retro): Cluster 114.0 + v114.0.0 tag prep` |

## Exit criteria

- `COVERAGE_MIN_LINES` raised in steps (11 → … → 40) — **met** (gate at 40 on full-suite coverage; measured ~60%).
- Fuzz/round-trip tests on the JSON-RPC / MCP envelope surface — **met**.
- `v114.0.0` tagged after retro.

## Ordering & risks

- **After the test clusters (111–113).** Builds on the established `proptest` pattern (112).
- **Risk — coverage-job runtime:** the instrumented full suite + testcontainers is heavier than the old unit-only run (timeout 45 → 75 min; first CI run completed in ~4 min with cache, 442 tests, 0 skipped). Coverage is a non-required check, so a hiccup doesn't block merges.
- **Risk — floor too tight:** set to 40 against a ~60% ceiling for ~20pt margin against CI variance.

## References

- [[Clusters/Product Ladder 102+]] Phase XXI
- [[Retros/Cluster 114.0]], [[Clusters/Cluster 112.0]] (proptest pattern)

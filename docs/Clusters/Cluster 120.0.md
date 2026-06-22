# Cluster 120.0 — Scale product gate

**Theme:** Verify and document the `maidan-scale-1.0` product gate; close Product Ladder 102+.

**Ladder:** [[Clusters/Product Ladder 102+]] Phase XXIII · tags **`v120.0.0`** + **`maidan-scale-1.0`**.

**Predecessor:** all of 102–119; the `maidan-operator-1.0` gate precedent (Cluster 101).

---

## Scope

| Layer | Deliverable |
|-------|-------------|
| **Gate e2e** | `maidan_scale_gate_e2e` — scale runtime surfaces + scale telemetry gauges respond. |
| **Gate doc** | `docs/Gates/maidan-scale-1.0.md` — 7 criteria → evidence + perf-budget approach + re-verify runbook. |
| **Baselines** | `STORE_BASELINE.md` recorded (pairs with `SEARCH_BASELINE.md`). |
| **CI** | Promote `scale-out smoke` to a gate-required check. |

## Non-goals

- New product capability — the gate is the conjunction of 102–119.
- Postgres SLA numbers (machine-specific; methodology documented).
- Edition 2024 adoption (Track V/X).

## PR ladder (actual)

| # | Title |
|---|--------|
| 120.0.1 | `feat(gate): maidan_scale_gate_e2e` (#327) |
| 120.0.2 | gate doc + `STORE_BASELINE.md` + scale-out promotion (#327) |
| 120.0.retro | `docs(retro): Cluster 120.0 + v120.0.0 / maidan-scale-1.0 tag prep` |

## Exit criteria

- `maidan-scale-1.0` e2e: multi-replica suite (102–105) + perf budgets (109) + coverage floor (114) + clean `cargo deny` + bench baselines recorded — **met** (gate doc maps each to evidence; e2e + baselines added).
- `v120.0.0` **and** `maidan-scale-1.0` tagged after retro — closes Phase XXIII and the 102+ ladder.

## Ordering & risks

- **120 last** (gates the whole ladder).
- **Risk — manual branch-protection promotion:** documented as an operator action; the job runs on every PR regardless.

## References

- [[Clusters/Product Ladder 102+]] Phase XXIII; [[Retros/Cluster 101.0]] (operator-gate precedent)
- [[Retros/Cluster 120.0]], [Gates/maidan-scale-1.0.md](../Gates/maidan-scale-1.0.md)

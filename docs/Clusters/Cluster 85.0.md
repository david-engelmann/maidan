# Cluster 85.0 — sqlite-vec optional

**Theme:** sqlite-vec optional.

**Ladder:** [[Clusters/Product Ladder 77+]] Phase XV · tag **`v85.0.0`**.

**Predecessor:** Cluster **75** semantic runbook.

---

## Problem

Deferred from [[Clusters/Product Ladder 77+]] and [[Remaining Work]] — see Phase XV row for cluster **85**.

---

## Scope

| Layer | Deliverable |
|-------|-------------|
| **Core** | Feature-gated HNSW on SQLite via optional `sqlite-vec`; CI proves linkage or documents opt-out. |
| **Tests** | e2e or store parity per cluster theme |
| **Docs** | Update [[Agent Integration]] / [[Production]] / [[Capabilities]] as needed |

---

## Non-goals

- Default-on sqlite-vec in all builds.

---

## PR ladder (suggested)

| # | Title |
|---|--------|
| 85.0.1 feat(search): optional sqlite-vec feature flag |
| 85.0.2 ci: sqlite-vec linkage job |
| 85.0.retro docs(retro): Cluster 85.0 + v85.0.0 tag prep |

---

## Exit criteria

- Exit line from [[Clusters/Product Ladder 77+]] met in code + tests.
- **`v85.0.0`** tagged after retro.

---

## References

- [[Clusters/Product Ladder 77+]], [[Clusters/Cluster 84.0]] (if applicable)
- [[Remaining Work]], [[Open Work]]

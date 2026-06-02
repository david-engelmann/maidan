# Cluster 83.0 — SQLite delivery cursor

**Theme:** SQLite delivery cursor.

**Ladder:** [[Clusters/Product Ladder 77+]] Phase XV · tag **`v83.0.0`**.

**Predecessor:** Cluster **13** Postgres cursor.

---

## Problem

Deferred from [[Clusters/Product Ladder 77+]] and [[Remaining Work]] — see Phase XV row for cluster **83**.

---

## Scope

| Layer | Deliverable |
|-------|-------------|
| **Core** | `maidan_delivery_cursor` implemented for SQLite; shared store tests with Postgres. |
| **Tests** | e2e or store parity per cluster theme |
| **Docs** | Update [[Agent Integration]] / [[Production]] / [[Capabilities]] as needed |

---

## Non-goals

- Postgres-only resume story.

---

## PR ladder (suggested)

| # | Title |
|---|--------|
| 83.0.1 feat(store): sqlite delivery cursor impl |
| 83.0.2 test(store): delivery_cursor parity |
| 83.0.retro docs(retro): Cluster 83.0 + v83.0.0 tag prep |

---

## Exit criteria

- Exit line from [[Clusters/Product Ladder 77+]] met in code + tests.
- **`v83.0.0`** tagged after retro.

---

## References

- [[Clusters/Product Ladder 77+]], [[Clusters/Cluster 82.0]] (if applicable)
- [[Remaining Work]], [[Open Work]]

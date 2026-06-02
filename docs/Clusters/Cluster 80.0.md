# Cluster 80.0 — Delivery ops unified

**Theme:** Delivery ops unified.

**Ladder:** [[Clusters/Product Ladder 77+]] Phase XIV · tag **`v80.0.0`**.

**Predecessor:** Cluster **68** automation DLQ + webhook worker.

---

## Problem

Deferred from [[Clusters/Product Ladder 77+]] and [[Remaining Work]] — see Phase XIV row for cluster **80**.

---

## Scope

| Layer | Deliverable |
|-------|-------------|
| **Core** | Single operator API to list/filter/replay webhook + automation deliveries (shared query shape; tables may stay separate). |
| **Tests** | e2e or store parity per cluster theme |
| **Docs** | Update [[Agent Integration]] / [[Production]] / [[Capabilities]] as needed |

---

## Non-goals

- Merged DB tables for webhooks and automation.

---

## PR ladder (suggested)

| # | Title |
|---|--------|
| 80.0.1 feat(server): unified delivery list API |
| 80.0.2 feat(server): unified replay route |
| 80.0.3 test(server): delivery_ops_unified e2e |
| 80.0.retro docs(retro): Cluster 80.0 + v80.0.0 tag prep |

---

## Exit criteria

- Exit line from [[Clusters/Product Ladder 77+]] met in code + tests.
- **`v80.0.0`** tagged after retro.

---

## References

- [[Clusters/Product Ladder 77+]], [[Clusters/Cluster 79.0]] (if applicable)
- [[Remaining Work]], [[Open Work]]

# Cluster 79.0 — A2A long-running tasks

**Theme:** A2A long-running tasks.

**Ladder:** [[Clusters/Product Ladder 77+]] Phase XIV · tag **`v79.0.0`**.

**Predecessor:** Cluster **72** persisted tasks + subscribe.

---

## Problem

Deferred from [[Clusters/Product Ladder 77+]] and [[Remaining Work]] — see Phase XIV row for cluster **79**.

---

## Scope

| Layer | Deliverable |
|-------|-------------|
| **Core** | `tasks/cancel` RPC, progress events on `SubscribeToTask`, terminal semantics documented and tested. |
| **Tests** | e2e or store parity per cluster theme |
| **Docs** | Update [[Agent Integration]] / [[Production]] / [[Capabilities]] as needed |

---

## Non-goals

- Full A2A task marketplace UX.

---

## PR ladder (suggested)

| # | Title |
|---|--------|
| 79.0.1 feat(a2a): task cancel + progress store fields |
| 79.0.2 feat(server): SubscribeToTask progress events |
| 79.0.3 test(server): a2a long_task e2e |
| 79.0.retro docs(retro): Cluster 79.0 + v79.0.0 tag prep |

---

## Exit criteria

- Exit line from [[Clusters/Product Ladder 77+]] met in code + tests.
- **`v79.0.0`** tagged after retro.

---

## References

- [[Clusters/Product Ladder 77+]], [[Clusters/Cluster 78.0]] (if applicable)
- [[Remaining Work]], [[Open Work]]

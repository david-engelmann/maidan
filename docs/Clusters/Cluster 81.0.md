# Cluster 81.0 — Subscribe grants v3

**Theme:** Subscribe grants v3.

**Ladder:** [[Clusters/Product Ladder 77+]] Phase XIV · tag **`v81.0.0`**.

**Predecessor:** Cluster **77** HTTP capability map.

---

## Problem

Deferred from [[Clusters/Product Ladder 77+]] and [[Remaining Work]] — see Phase XIV row for cluster **81**.

---

## Scope

| Layer | Deliverable |
|-------|-------------|
| **Core** | WS filter schema requires explicit channel grants; private-channel deny e2e. |
| **Tests** | e2e or store parity per cluster theme |
| **Docs** | Update [[Agent Integration]] / [[Production]] / [[Capabilities]] as needed |

---

## Non-goals

- Org-wide default-allow subscribe.

---

## PR ladder (suggested)

| # | Title |
|---|--------|
| 81.0.1 feat(contracts): ws-subscribe-filter grants schema |
| 81.0.2 feat(server): enforce channel grants on subscribe |
| 81.0.3 test(server): subscribe_grants_e2e |
| 81.0.retro docs(retro): Cluster 81.0 + v81.0.0 tag prep |

---

## Exit criteria

- Exit line from [[Clusters/Product Ladder 77+]] met in code + tests.
- **`v81.0.0`** tagged after retro.

---

## References

- [[Clusters/Product Ladder 77+]], [[Clusters/Cluster 80.0]] (if applicable)
- [[Remaining Work]], [[Open Work]]

# Cluster 88.0 — Helm production profiles

**Theme:** Helm production profiles.

**Ladder:** [[Clusters/Product Ladder 77+]] Phase XVI · tag **`v88.0.0`**.

**Predecessor:** Cluster **55** helm stack.

---

## Problem

Deferred from [[Clusters/Product Ladder 77+]] and [[Remaining Work]] — see Phase XVI row for cluster **88**.

---

## Scope

| Layer | Deliverable |
|-------|-------------|
| **Core** | Documented Helm values overlays: external OTel, Redis quotas, S3, ingress TLS. |
| **Tests** | e2e or store parity per cluster theme |
| **Docs** | Update [[Agent Integration]] / [[Production]] / [[Capabilities]] as needed |

---

## Non-goals

- Hosted Helm operator.

---

## PR ladder (suggested)

| # | Title |
|---|--------|
| 88.0.1 docs(helm): production values profiles |
| 88.0.2 ci: helm profile smoke |
| 88.0.retro docs(retro): Cluster 88.0 + v88.0.0 tag prep |

---

## Exit criteria

- Exit line from [[Clusters/Product Ladder 77+]] met in code + tests.
- **`v88.0.0`** tagged after retro.

---

## References

- [[Clusters/Product Ladder 77+]], [[Clusters/Cluster 87.0]] (if applicable)
- [[Remaining Work]], [[Open Work]]

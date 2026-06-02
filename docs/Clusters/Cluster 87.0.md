# Cluster 87.0 — Reindex job API

**Theme:** Reindex job API.

**Ladder:** [[Clusters/Product Ladder 77+]] Phase XVI · tag **`v87.0.0`**.

**Predecessor:** Cluster **75** CLI reindex.

---

## Problem

Deferred from [[Clusters/Product Ladder 77+]] and [[Remaining Work]] — see Phase XVI row for cluster **87**.

---

## Scope

| Layer | Deliverable |
|-------|-------------|
| **Core** | `POST` operator route to enqueue embedding reindex + job status polling. |
| **Tests** | e2e or store parity per cluster theme |
| **Docs** | Update [[Agent Integration]] / [[Production]] / [[Capabilities]] as needed |

---

## Non-goals

- Distributed reindex workers.

---

## PR ladder (suggested)

| # | Title |
|---|--------|
| 87.0.1 feat(server): reindex embeddings job API |
| 87.0.2 test(server): reindex_job_e2e |
| 87.0.retro docs(retro): Cluster 87.0 + v87.0.0 tag prep |

---

## Exit criteria

- Exit line from [[Clusters/Product Ladder 77+]] met in code + tests.
- **`v87.0.0`** tagged after retro.

---

## References

- [[Clusters/Product Ladder 77+]], [[Clusters/Cluster 86.0]] (if applicable)
- [[Remaining Work]], [[Open Work]]

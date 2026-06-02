# Cluster 86.0 — Per-model embeddings

**Theme:** Per-model embeddings.

**Ladder:** [[Clusters/Product Ladder 77+]] Phase XV · tag **`v86.0.0`**.

**Predecessor:** Cluster **85** optional vec.

---

## Problem

Deferred from [[Clusters/Product Ladder 77+]] and [[Remaining Work]] — see Phase XV row for cluster **86**.

---

## Scope

| Layer | Deliverable |
|-------|-------------|
| **Core** | Schema split by `embedding_model`; queries filter by model at index time. |
| **Tests** | e2e or store parity per cluster theme |
| **Docs** | Update [[Agent Integration]] / [[Production]] / [[Capabilities]] as needed |

---

## Non-goals

- Online model migration without reindex.

---

## PR ladder (suggested)

| # | Title |
|---|--------|
| 86.0.1 feat(store): per-model embedding tables |
| 86.0.2 feat(search): model-scoped queries |
| 86.0.3 test(search): per_model_embeddings |
| 86.0.retro docs(retro): Cluster 86.0 + v86.0.0 tag prep |

---

## Exit criteria

- Exit line from [[Clusters/Product Ladder 77+]] met in code + tests.
- **`v86.0.0`** tagged after retro.

---

## References

- [[Clusters/Product Ladder 77+]], [[Clusters/Cluster 85.0]] (if applicable)
- [[Remaining Work]], [[Open Work]]

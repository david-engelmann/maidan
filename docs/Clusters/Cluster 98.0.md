# Cluster 98.0 — Mention notifications

**Theme:** Mention notifications.

**Ladder:** [[Clusters/Product Ladder 77+]] Phase XVIII · tag **`v98.0.0`**.

**Predecessor:** Cluster **50** webhooks.

---

## Problem

Deferred from [[Clusters/Product Ladder 77+]] and [[Remaining Work]] — see Phase XVIII row for cluster **98**.

---

## Scope

| Layer | Deliverable |
|-------|-------------|
| **Core** | Mention → webhook router (per-workspace config); no email required. |
| **Tests** | e2e or store parity per cluster theme |
| **Docs** | Update [[Agent Integration]] / [[Production]] / [[Capabilities]] as needed |

---

## Non-goals

- Email digests.

---

## PR ladder (suggested)

| # | Title |
|---|--------|
| 98.0.1 feat(server): mention notification router |
| 98.0.2 test(server): mention_notify_e2e |
| 98.0.retro docs(retro): Cluster 98.0 + v98.0.0 tag prep |

---

## Exit criteria

- Exit line from [[Clusters/Product Ladder 77+]] met in code + tests.
- **`v98.0.0`** tagged after retro.

---

## References

- [[Clusters/Product Ladder 77+]], [[Clusters/Cluster 97.0]] (if applicable)
- [[Remaining Work]], [[Open Work]]

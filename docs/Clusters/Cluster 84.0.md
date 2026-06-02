# Cluster 84.0 — Outbox relay modes

**Theme:** Outbox relay modes.

**Ladder:** [[Clusters/Product Ladder 77+]] Phase XV · tag **`v84.0.0`**.

**Predecessor:** Cluster **56** outbox replay HTTP.

---

## Problem

Deferred from [[Clusters/Product Ladder 77+]] and [[Remaining Work]] — see Phase XV row for cluster **84**.

---

## Scope

| Layer | Deliverable |
|-------|-------------|
| **Core** | Configurable polled relay mode + runbook for NOTIFY loss; prod must not silently downgrade. |
| **Tests** | e2e or store parity per cluster theme |
| **Docs** | Update [[Agent Integration]] / [[Production]] / [[Capabilities]] as needed |

---

## Non-goals

- Exactly-once NOTIFY.

---

## PR ladder (suggested)

| # | Title |
|---|--------|
| 84.0.1 feat(server): outbox polled relay flag |
| 84.0.2 docs: Production outbox relay runbook |
| 84.0.3 test(server): outbox_polled_relay e2e |
| 84.0.retro docs(retro): Cluster 84.0 + v84.0.0 tag prep |

---

## Exit criteria

- Exit line from [[Clusters/Product Ladder 77+]] met in code + tests.
- **`v84.0.0`** tagged after retro.

---

## References

- [[Clusters/Product Ladder 77+]], [[Clusters/Cluster 83.0]] (if applicable)
- [[Remaining Work]], [[Open Work]]

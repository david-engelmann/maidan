# Cluster 90.0 — SLO alert templates

**Theme:** SLO alert templates.

**Ladder:** [[Clusters/Product Ladder 77+]] Phase XVI · tag **`v90.0.0`**.

**Predecessor:** Cluster **89** OTLP export.

---

## Problem

Deferred from [[Clusters/Product Ladder 77+]] and [[Remaining Work]] — see Phase XVI row for cluster **90**.

---

## Scope

| Layer | Deliverable |
|-------|-------------|
| **Core** | Grafana/Alertmanager templates for agent latency, DLQ depth, outbox lag. |
| **Tests** | e2e or store parity per cluster theme |
| **Docs** | Update [[Agent Integration]] / [[Production]] / [[Capabilities]] as needed |

---

## Non-goals

- PagerDuty integrations.

---

## PR ladder (suggested)

| # | Title |
|---|--------|
| 90.0.1 docs(ops): SLO alert templates |
| 90.0.retro docs(retro): Cluster 90.0 + v90.0.0 tag prep |

---

## Exit criteria

- Exit line from [[Clusters/Product Ladder 77+]] met in code + tests.
- **`v90.0.0`** tagged after retro.

---

## References

- [[Clusters/Product Ladder 77+]], [[Clusters/Cluster 89.0]] (if applicable)
- [[Remaining Work]], [[Open Work]]

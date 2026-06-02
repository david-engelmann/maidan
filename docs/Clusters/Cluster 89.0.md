# Cluster 89.0 — OTLP metrics export

**Theme:** OTLP metrics export.

**Ladder:** [[Clusters/Product Ladder 77+]] Phase XVI · tag **`v89.0.0`**.

**Predecessor:** Cluster **76** metrics runbook.

---

## Problem

Deferred from [[Clusters/Product Ladder 77+]] and [[Remaining Work]] — see Phase XVI row for cluster **89**.

---

## Scope

| Layer | Deliverable |
|-------|-------------|
| **Core** | Prometheus scrape or OTLP push from server; example dashboard JSON in repo. |
| **Tests** | e2e or store parity per cluster theme |
| **Docs** | Update [[Agent Integration]] / [[Production]] / [[Capabilities]] as needed |

---

## Non-goals

- Hosted Grafana Cloud bundle.

---

## PR ladder (suggested)

| # | Title |
|---|--------|
| 89.0.1 feat(observability): OTLP metrics export |
| 89.0.2 docs: example Grafana dashboard JSON |
| 89.0.retro docs(retro): Cluster 89.0 + v89.0.0 tag prep |

---

## Exit criteria

- Exit line from [[Clusters/Product Ladder 77+]] met in code + tests.
- **`v89.0.0`** tagged after retro.

---

## References

- [[Clusters/Product Ladder 77+]], [[Clusters/Cluster 88.0]] (if applicable)
- [[Remaining Work]], [[Open Work]]

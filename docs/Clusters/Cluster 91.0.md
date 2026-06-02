# Cluster 91.0 — Bootstrap compile-time strip

**Theme:** Bootstrap compile-time strip.

**Ladder:** [[Clusters/Product Ladder 77+]] Phase XVI · tag **`v91.0.0`**.

**Predecessor:** Track V threat model.

---

## Problem

Deferred from [[Clusters/Product Ladder 77+]] and [[Remaining Work]] — see Phase XVI row for cluster **91**.

---

## Scope

| Layer | Deliverable |
|-------|-------------|
| **Core** | Release build without bootstrap routes via Cargo feature; [[Threat-Model]] updated. |
| **Tests** | e2e or store parity per cluster theme |
| **Docs** | Update [[Agent Integration]] / [[Production]] / [[Capabilities]] as needed |

---

## Non-goals

- Runtime-only bootstrap disable.

---

## PR ladder (suggested)

| # | Title |
|---|--------|
| 91.0.1 feat(server): bootstrap compile-time feature |
| 91.0.2 docs: Threat-Model bootstrap strip |
| 91.0.retro docs(retro): Cluster 91.0 + v91.0.0 tag prep |

---

## Exit criteria

- Exit line from [[Clusters/Product Ladder 77+]] met in code + tests.
- **`v91.0.0`** tagged after retro.

---

## References

- [[Clusters/Product Ladder 77+]], [[Clusters/Cluster 90.0]] (if applicable)
- [[Remaining Work]], [[Open Work]]

# Cluster 82.0 — Context pagination

**Theme:** Context pagination.

**Ladder:** [[Clusters/Product Ladder 77+]] Phase XV · tag **`v82.0.0`**.

**Predecessor:** Cluster **74** MCP context tools.

---

## Problem

Deferred from [[Clusters/Product Ladder 77+]] and [[Remaining Work]] — see Phase XV row for cluster **82**.

---

## Scope

| Layer | Deliverable |
|-------|-------------|
| **Core** | HTTP + MCP context tools accept cursor/limit; stable ordering documented. |
| **Tests** | e2e or store parity per cluster theme |
| **Docs** | Update [[Agent Integration]] / [[Production]] / [[Capabilities]] as needed |

---

## Non-goals

- Unbounded context export.

---

## PR ladder (suggested)

| # | Title |
|---|--------|
| 82.0.1 feat(types): context cursor types |
| 82.0.2 feat(server): paginated context HTTP + MCP |
| 82.0.3 test(server): context_pagination e2e |
| 82.0.retro docs(retro): Cluster 82.0 + v82.0.0 tag prep |

---

## Exit criteria

- Exit line from [[Clusters/Product Ladder 77+]] met in code + tests.
- **`v82.0.0`** tagged after retro.

---

## References

- [[Clusters/Product Ladder 77+]], [[Clusters/Cluster 81.0]] (if applicable)
- [[Remaining Work]], [[Open Work]]

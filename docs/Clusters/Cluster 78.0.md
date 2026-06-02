# Cluster 78.0 — MCP streamable bidirectional

**Theme:** MCP streamable bidirectional.

**Ladder:** [[Clusters/Product Ladder 77+]] Phase XIV · tag **`v78.0.0`**.

**Predecessor:** Cluster **73** streamable POST/DELETE baseline.

---

## Problem

Cluster **73** shipped streamable open/follow-up/DELETE, but follow-up `POST /mcp/streamable` returns JSON while the spec subset expects multiplexed responses on the SSE leg for an open session.

---

## Scope

| Layer | Deliverable |
|-------|-------------|
| **Core** | Follow-up JSON-RPC on an open `Mcp-Session-Id` session delivers responses on the SSE stream (2024-11-05 subset); [[Agent Integration]] documents client flow. |
| **Tests** | e2e or store parity per cluster theme |
| **Docs** | Update [[Agent Integration]] / [[Production]] / [[Capabilities]] as needed |

---

## Non-goals

- Full spec-complete streamable transport (every MCP edge case).

---

## PR ladder (suggested)

| # | Title |
|---|--------|
| 78.0.1 feat(mcp): streamable follow-up responses on SSE mux |
| 78.0.2 test(server): mcp_streamable bidirectional e2e |
| 78.0.3 docs: Agent Integration streamable client flow |
| 78.0.retro docs(retro): Cluster 78.0 + v78.0.0 tag prep |

---

## Exit criteria

- Exit line from [[Clusters/Product Ladder 77+]] met in code + tests.
- **`v78.0.0`** tagged after retro.

---

## References

- [[Clusters/Product Ladder 77+]], [[Clusters/Cluster 77.0]] (if applicable)
- [[Remaining Work]], [[Open Work]]

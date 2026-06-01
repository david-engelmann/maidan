# Cluster 69.0 — Capabilities matrix complete

**Theme:** Finish Cluster **63** kickoff — every MCP tool and critical HTTP route has tested capability enforcement.

## Problem

[[Clusters/Cluster 22.0]] established denial tests for a handful of paths. Cluster **63** aimed for full matrix coverage; **`v63.0.0`** shipped only one additional MCP denial case in `agent_surfaces_e2e`. Agents still hit surprising JSON-RPC `-32003` / HTTP 403 in production.

`capability_matrix_e2e.rs` already covers search, `post_message`, A2A `SendMessage`, artifact upload, and WS subscribe — **not** the full MCP catalog in `contracts/mcp-tool-names.json`.

## Scope

| Layer | Deliverable |
|-------|-------------|
| Docs | `docs/Capability Map.md` — tool/route → required capability strings |
| Tests | Table-driven `capability_matrix_e2e` (or split `mcp_capability_matrix_e2e`) generated from map + contract file |
| Fixes | Close any gaps found (routes/tools that skip `AuthContext` checks) |
| CI | Fail if MCP catalog or OpenAPI paths lack a map entry |

## Non-goals

- New capabilities strings (use existing `maidan-auth` constants).
- Per-app capability templates (Cluster **57** scope).

## PR ladder (suggested)

| # | Title |
|---|--------|
| 69.0.1 | `docs: capability map for HTTP MCP WS` |
| 69.0.2 | `test(server): table-driven MCP capability matrix` |
| 69.0.3 | `test(server): HTTP route capability matrix` |
| 69.0.4 | `fix(server): close capability gaps from matrix` |
| 69.0.retro | `docs(retro): Cluster 69.0 + v69.0.0 tag prep` |

## Exit criteria

- Every name in `contracts/mcp-tool-names.json` has deny + allow cases in CI.
- Documented map matches `maidan-mcp` and `routes.rs`.
- `v69.0.0` tagged after retro.

## References

- [[Clusters/Product Ladder 68+]], [[Clusters/Cluster 22.0]], [[Retros/Cluster 26.0]]

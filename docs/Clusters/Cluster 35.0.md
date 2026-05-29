# Cluster 35.0 — MCP streamable bidirectional mux

**Theme:** Complete the MCP 2024-11-05 streamable HTTP subset started in Clusters 27 and 34.

## Problem

`v34.0.0` adds `Mcp-Session-Id` correlation but clients cannot send follow-up JSON-RPC on the
same streamable session. Compliant agent runtimes expect bidirectional mux on one HTTP connection.

## Scope

| Layer | Deliverable |
|-------|-------------|
| Server | Session-scoped channel: after first `POST /mcp/streamable`, accept additional JSON-RPC on same `Mcp-Session-Id` (design: long-poll or secondary frame channel — document choice in PR) |
| MCP | Session registry with TTL; graceful close |
| Tests | E2E: initialize + tools/list on same session id |
| Docs | OpenAPI + mcp-reference; update [[Remaining Work]] streamable row |

## Out of scope

- Replacing `GET /mcp/notifications` (keep v16 compat)
- WebSocket transport changes

## Tag

`v35.0.0`

## Depends on

Clusters 27, 34 (`POST /mcp/streamable`, `Mcp-Session-Id`).

See [[Clusters/Product Ladder 35+]] Phase I.

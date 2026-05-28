# Cluster 27.0 — MCP streamable HTTP multiplexing

Cluster 16.0 shipped `GET /mcp/notifications` SSE alongside request/response
`POST /mcp` at **`v16.0.0`**. That is a Maidan subset, not the full MCP
**streamable HTTP** transport (session-scoped bidirectional JSON-RPC on one HTTP
connection).

> **Goal:** Implement MCP streamable HTTP multiplexing so compliant HTTP clients
> receive responses and `notifications/resources/updated` (and future MCP
> notifications) on the same session without a separate notification-only GET.
>
> **Target tag:** `v27.0.0`.

## PRs

| #          | Title                                                                  | Issue |
|------------|------------------------------------------------------------------------|-------|
| kickoff    | `docs: Cluster 27.0 kickoff` (this doc)                                  | —     |
| 27.0.1     | `feat(maidan-server): MCP streamable HTTP session endpoint`             | TBD   |
| 27.0.2     | `feat(maidan-mcp): session bridge + notification mux`                    | TBD   |
| 27.0.3     | `test: streamable HTTP MCP integration`                                | TBD   |
| 27.0.4     | `docs: streamable HTTP in Architecture + MCP reference`                | TBD   |
| 27.0.retro | `docs(retro): Cluster 27.0 + v27.0.0 tag prep`                         | TBD   |

## Exit criteria

- HTTP MCP client using streamable HTTP receives JSON-RPC responses and at least
  one `notifications/resources/updated` on the same session.
- `GET /mcp/notifications` remains supported (Cluster 16 compatibility).
- `v27.0.0` tagged after retro.

## Out of scope

- Replacing `/mcp/stream` workspace bus SSE.
- WebSocket transport changes.

## References

- [[Clusters/Cluster 16.0]], [[Clusters/Product Ladder 17-27]].

# Cluster 16.0 retro — MCP HTTP resource notifications

> Closing wave for Cluster 16.0 · target tag `v16.0.0`.

Cluster 16.0 closed HTTP parity for MCP `resources/subscribe` without implementing
the full streamable HTTP spec or changing workspace event SSE.

## What shipped

- **PR #184** — Implementation bundle (16.0.1–16.0.4):
  - Shared `McpServer` on `AppState` (subscriptions persist across `POST /mcp`).
  - Broadcast fan-out for `notifications/resources/updated`.
  - `GET /mcp/notifications` SSE (JSON-RPC notification lines).
  - E2E: `http_resource_subscribe_delivers_sse_notification`.
  - Architecture, Decisions, Production, MCP reference updates.

## What was deferred

| To          | What                                              | Why                                      |
|-------------|---------------------------------------------------|------------------------------------------|
| Cluster 17  | Broader resource fan-out beyond `post_message`    | Scoped HTTP transport first.             |
| Post-16.0   | Full MCP streamable HTTP multiplexing             | Maidan SSE subset sufficient for agents. |
| Post-16.0   | Per-bearer MCP session isolation                  | Single shared dispatcher OK for MVP.     |

## Surprises

- Per-request `McpServer` construction made HTTP subscribe a silent no-op until
  state was lifted to `AppState`.

## Decisions

- **`GET /mcp/notifications`** — dedicated SSE; do not overload `/mcp/stream`.
- **Broadcast + stdio drain** — same queue path as Cluster 15 for stdio.

## Capability table extension

| Capability                                              | First available in |
|---------------------------------------------------------|--------------------|
| Shared MCP dispatcher on HTTP                             | `v16.0.0`          |
| `GET /mcp/notifications` SSE                              | `v16.0.0`          |

## Risks identified + still open

- **Fan-out completeness** — still centered on `post_message` until Cluster 17.
- **Multi-tenant isolation** — one process-wide subscription set.

## Forward look

Next: **Cluster 17.0** — MCP resource fan-out for mutations. Epic map:
[[Clusters/Product Ladder 17-26]].

## Acknowledgements

Cluster 15 stdio subscribe made HTTP transport a thin layer.

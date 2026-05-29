# Cluster 34.0 retro — MCP streamable session

> Tag **`v34.0.0`**.

## What shipped

- `Mcp-Session-Id` response header on `POST /mcp/streamable`; server tracks session ids.
- Clients may pass existing `Mcp-Session-Id` to reuse correlation id.

## What was deferred

- Follow-up JSON-RPC frames on the same HTTP connection after SSE opens.
- `Mcp-Session-Id` on `GET /mcp/notifications`.

## Forward look

Post-ladder backlog: full streamable mux, UI depth, `mcp-stdio` Postgres.

# Cluster 35.0 retro — MCP streamable bidirectional mux

> Tag **`v35.0.0`**.

## What shipped

- `StreamableSessionRegistry` in `maidan-mcp`: open session, push SSE frames, close on disconnect.
- First `POST /mcp/streamable` opens SSE + `Mcp-Session-Id`; live notifications fan into the session.
- Follow-up `POST /mcp/streamable` with open `Mcp-Session-Id` returns JSON-RPC directly and pushes the same frame to the SSE consumer.
- E2E: `initialize` then `tools/list` on the same session id.

## What was deferred

- Session TTL / idle sweep (sessions close when SSE client disconnects only).
- `Mcp-Session-Id` on `GET /mcp/notifications`.
- Full MCP 2024-11-05 streamable transport (single long-lived HTTP connection for both directions).

## Forward look

Cluster **36**: `maidan-cli mcp-stdio` against Postgres `DATABASE_URL`.

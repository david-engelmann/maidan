# Cluster H — Web UI, MCP stdio, production polish

After Cluster G federated deployments, Cluster H makes Maidan usable by
humans and desktop MCP clients: a minimal web UI, stdio MCP transport,
SSE event stream for reactive clients, and production ergonomics.

> **Goal:** Operators browse workspaces in a browser; Claude Desktop-style
> clients talk MCP over stdio; HTTP clients can subscribe to events via SSE;
> the server shuts down cleanly on SIGINT.
>
> **Target tag:** `v0.7.0`.

## PRs

| #       | Title                                                                 | Issue |
|---------|-----------------------------------------------------------------------|-------|
| H.1     | `feat(maidan-server): graceful shutdown + request-id middleware`      | TBD   |
| H.2     | `feat(maidan-cli): MCP stdio transport`                               | TBD   |
| H.3     | `feat(maidan-server): MCP SSE event stream GET /mcp/stream`           | TBD   |
| H.4     | `feat(maidan-server): minimal web UI at /ui`                          | TBD   |
| H.retro | `docs(retro): Cluster H retrospective + v0.7.0 tag prep`            | TBD   |

## Order

1. **H.1** — `tokio::select!` on ctrl_c; drain indexer + federation worker;
   `X-Request-Id` on every response (generate if missing).
2. **H.2** — `maidan-cli mcp-stdio`: line-delimited JSON-RPC on stdin/stdout;
   wires `McpServer` + store from `DATABASE_URL` (same env as server).
3. **H.3** — `GET /mcp/stream?filter=…` with Bearer auth + `event:subscribe`;
   `text/event-stream` frames mirroring WebSocket payload shape.
4. **H.4** — static `ui/index.html` served under `/ui/`; workspace picker,
   channel list, event tail (read-only v0.7.0).
5. **H.retro** + `v0.7.0` tag.

## Exit criteria

- CI green on `main`.
- `maidan-cli mcp-stdio` handles `initialize` + `tools/list` against SQLite memory DB.
- Browser loads `/ui/` and lists events when `AUTH_DISABLED=1` or token provided.
- SIGINT stops server without orphan tasks (indexer + federation worker shut down).
- [[Retros/Cluster H]] merged; `v0.7.0` tagged.

## Risks

| Risk                              | Mitigation                                      |
|-----------------------------------|-------------------------------------------------|
| UI auth story awkward in browser  | v0.7.0: prompt for bearer token in localStorage |
| SSE vs full MCP streamable HTTP   | Document as Maidan SSE subset; not full spec    |
| Static UI drift from API          | UI uses same JSON shapes as public HTTP routes  |

## Out of scope (deferred)

- Full mdBook/docs site (Track W).
- Faceted search UI filters (post-1.0 polish).
- React/Vite build pipeline (vanilla static v0.7.0).
- Helm chart (Cluster A deferral).

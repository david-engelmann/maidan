# Cluster 27.0 retro — MCP streamable HTTP multiplexing

> Closing wave for Cluster 27.0 · target tag **`v27.0.0`** (culminating Product Ladder 17–27 release).

Cluster 27.0 closed the product ladder transport gap with HTTP streamable MCP
(response + notifications on one SSE body). Clusters **23–26** shipped in the same
integration PR; CHANGELOG sections `v23.0.0`–`v26.0.0` record logical boundaries.

## What shipped

- **PR #198** (`0cffd8f`) — `POST /mcp/streamable`: JSON-RPC response as first SSE event,
  pending + live `notifications/resources/updated` on same stream; `mcp_streamable_e2e`;
  MCP reference + OpenAPI notes; `GET /mcp/notifications` unchanged (v16 compat).
- **PR #198** — [[Remaining Work]] register + vault refresh (post-ladder backlog).

## What was deferred

| To | What | Why |
|----|------|-----|
| [[Remaining Work]] | Full MCP spec bidirectional session on one HTTP connection | Shipped POST+SSE subset, not session-scoped client→server mux. |
| Post-27 | `Mcp-Session-Id` header bridge | Spec-complete streamable HTTP follow-up. |
| Post-21 | A2A `SendStreamingMessage` | Separate transport. |

## Surprises

- `BroadcastStream` + `chain` typing required mpsc-backed SSE in `mcp_streamable.rs.

## Decisions

- **One integration PR for 23–27** — acceptable stopping point; single retro wave and
  **`v27.0.0`** tag triggers one GitHub Release (CHANGELOG still lists v23–v26).

## Capability table extension

| Capability | First available in |
|------------|-------------------|
| MCP streamable HTTP subset (`POST /mcp/streamable`) | `v27.0.0` |

## Risks identified + still open

- Compliant MCP HTTP clients expecting full streamable transport may need dual
  `POST /mcp` + `GET /mcp/notifications` until session mux ships.

## Forward look

Product Ladder **17–27** is complete. Next work: [[Remaining Work]] (Slack parity,
full erasure, Helm umbrella, UI depth, ops hardening). No numbered cluster **28**
defined yet.

## Acknowledgements

- Maintainer merge #198.

# Cluster 145.0 — MCP conformance basics (initialize/version + batching + notifications)

**Theme:** First slice of the **MCP streamable transport spec-completeness arc
(145–148)**. Close the JSON-RPC / lifecycle conformance gaps that don't need
transport rework, before the streamable-specific work.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v145.0.0`**, no new gate tag.

---

## The arc (145–148)

The backlog's "full MCP streamable 2024-11-05 bidirectional mux / spec-complete
session" is really **5 gaps** (see the Cluster 35.0 retro for the shipped
subset). Delivered across four clusters, easiest→hardest:

| Cluster | Gap |
|---------|-----|
| **145** (this) | `initialize` param/version negotiation; `MCP-Protocol-Version` header; JSON-RPC batching; notifications |
| 146 | `GET /mcp/streamable` server→client SSE stream + `Accept`-header content negotiation |
| 147 | Resumability (`Last-Event-ID` replay, SSE `id:`) |
| 148 | Server→client requests (sampling / roots / elicitation) + per-session client-capability tracking |

## Scope (145)

| Change | Where |
|--------|-------|
| `initialize` reads params + negotiates `protocolVersion`; `SUPPORTED_PROTOCOL_VERSIONS` API | `maidan-mcp/src/server.rs`, `lib.rs` |
| `MCP-Protocol-Version` header validation (400 on unsupported) | `maidan-server/src/mcp.rs` (shared helper), `mcp_streamable.rs` |
| JSON-RPC batching (array → array) + empty-batch `-32600` | `maidan-server/src/mcp.rs` |
| Notifications → `202` no body; `notifications/initialized`/`cancelled` accepted | `mcp.rs`, `server.rs` dispatch |

## Non-goals (deferred within the arc)

- Streamable-transport batching over SSE — 146 (needs the muxing path).
- Per-session client-capability storage — 148 (server→client requests need it;
  145 negotiates the version but doesn't yet persist the client's declared
  capabilities).
- Adding protocol revisions beyond `2024-11-05` to the supported set — done as
  each revision's transport features land (146–148).

## PR ladder (actual)

| # | Title |
|---|--------|
| 145.0.1 | `feat(mcp): initialize/version negotiation + JSON-RPC batching + notifications` (#380) |
| 145.0.retro | `docs(retro): Cluster 145.0 + v145.0.0 tag prep` |

## Exit criteria

- `initialize` negotiates; header validated; batches + notifications handled;
  tests green — **met**.
- `v145.0.0` tagged after retro.

## Verification & limits

- Unit (maidan-mcp): negotiation rule + notification acceptance. E2E (mcp_e2e):
  batch→array, notification→202, unsupported-header→400, initialize negotiation.
  `fmt`/`clippy` clean; streamable/capability-matrix/stdio e2e unaffected.

## References

- [[Retros/Cluster 145.0]]; [[Retros/Cluster 35.0]] (the shipped subset);
  `maidan-mcp/src/server.rs`, `maidan-server/src/mcp.rs`.

# Cluster 148.0 retro — MCP server→client requests (arc 145–148 conclusion)

> Tag **`v148.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> **Concludes the MCP streamable spec-completeness arc (145–148).**

## What shipped

- **Per-session client-capability tracking** — the streamable handler records
  the client's `capabilities` from `initialize`; `client_supports` gates
  server→client requests.
- **`McpServer::request_client(session, method, params)`** — issues a
  server→client JSON-RPC request (`sampling/createMessage`, `roots/list`,
  `elicitation/create`), gated on the declared capability (else `Forbidden`);
  allocates an id + pending oneshot, pushes onto the session SSE, awaits the
  response (30s timeout).
- **Inbound response routing** — `POST /mcp/streamable` tells a JSON-RPC
  response (has `id`, no `method`) from a request and routes it to the awaiting
  caller (`resolve_client_response`), returning `202`.

## What was deferred / not covered

| To | What | Why |
|----|------|-----|
| Future | An organic caller for `request_client` | Maidan's server has no feature that samples/roots/elicits yet; this is transport capability, ready for one. |
| Future | Batched client responses on the streamable endpoint | Single responses routed; batch-of-responses can follow if a client sends them. |

## Surprises

- **The transport already had everything.** The session SSE leg carries the
  outbound request; the correlation is a oneshot in the session entry; the only
  new inbound wiring is a request-vs-response discriminant (`method` present?).
- **Client capabilities were being discarded since forever** (noted back in
  145) — this cluster is the first to store and act on them.

## Decisions

- **Capture capabilities in the handler, not the dispatcher** — keeps
  `McpServer::handle` transport-agnostic; the session id (needed to key the
  capabilities) only exists at the transport layer.
- **Build the machinery despite no caller** — the arc's explicit goal was
  literal spec-completeness; the capability is implemented, gated, and tested
  end to end rather than stubbed.

## Capability table extension

| Capability | Where |
|------------|-------|
| MCP server→client requests (sampling/roots/elicitation) + client-capability gating | `maidan-mcp/src/server.rs`, `streamable_session.rs`, `mcp_streamable.rs` |

## Risks identified + still open

- **Unused surface.** `request_client` has no in-tree caller, so its real-world
  behavior is exercised only by tests. Documented as intentional (transport
  capability for future features), not a stub.

## Forward look

The **MCP streamable spec-completeness backlog item is closed** (arc 145–148:
version negotiation, header, batching, notifications, GET SSE, `Accept`,
resumability, and now bidirectional requests). The remaining backlog is UX
polish and out-of-scope items — no open *backend* capability gaps.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Requested end to end by
the maintainer ("complete full MCP streamable spec-completeness").

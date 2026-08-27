# Cluster 301.0 retro — MCP `2026-07-28` stateless streamable core

> Tag **`v301.0.0`**. Phase XXIV (post-gate hardening). MCP `2026-07-28` arc, part 2 (J3.3–4). No new gate tag.

## What shipped

The stateless core of the `2026-07-28` revision on `POST /mcp/streamable`:

- **A `2026-07-28` request lands cold.** When the POST carries `MCP-Protocol-Version:
  2026-07-28`, `streamable` now serves it inline (`handle_in_session(request, auth, None)`) and
  returns a single JSON-RPC response — **never minting or requiring an `Mcp-Session-Id`**,
  regardless of `Accept`. Sessions were removed in `2026-07-28`, so "any request lands on any
  server instance" holds. New `crate::mcp::is_stateless_request(headers)` +
  `STATELESS_PROTOCOL_VERSION` gate it.
- **The `2024-11-05` SSE-session path is untouched.** A 2024 client that accepts SSE and omits
  a session still gets `open_new_streamable_session` (a fresh `Mcp-Session-Id` + SSE), exactly
  as before. The stateless branch sits ahead of it, keyed only on the 2026 header.
- **Live-wait stays where it belongs (J3.4).** Server→client requests and live-wait ride
  `GET /mcp/stream` / WS / the `wait_for_*` tools — a 2026 client is not told that a POST GET-session
  *is* Streamable HTTP. The 2024 session machinery is retained only for 2024 clients.

## Surprises / decisions

- **`POST /mcp` was already stateless.** The non-streamable transport (`mcp.rs`) is pure
  JSON-RPC-in → JSON-RPC-out with no session, so a 2026 client using `POST /mcp` already worked.
  The only session-minting path was `POST /mcp/streamable` with `Accept: text/event-stream` +
  no session id → this cluster closes exactly that gap for 2026.
- **Inline JSON, not a one-shot SSE.** For a 2026 request the server returns `application/json`
  even when the client sent `Accept: text/event-stream`. Maidan's tool responses are
  single-shot (no partial streaming), the spec permits either body, and JSON is unambiguously
  sessionless. A one-shot SSE would respect `Accept` more literally but adds machinery for no
  behavioral gain here; logged as an optional refinement.
- **Known limitation (documented):** the `request_client` server→client tools
  (`summarize_thread`, `request_approval`, `list_roots`) still rely on the 2024 streamable
  session for delivery, so a *stateless* 2026 client can't use them over the POST — it uses
  `GET /mcp/stream` for live-wait. Capturing `_meta.io.modelcontextprotocol/clientInfo`
  per-request (for stateless server→client) is a niche follow-up, not core tools/call.

## Capability table extension

`POST /mcp/streamable` now serves a `2026-07-28` request statelessly (inline JSON, no
`Mcp-Session-Id`); the 2024 SSE-session path is unchanged. No new tool/capability.

## Risks identified + still open

- Stateless server→client (`request_client`) is out of scope — those tools need the 2024
  session or ride `GET /mcp/stream`. Fine for the core "cold `tools/call`" contract.

## Forward look

**302** — `Mcp-Method` / `Mcp-Name` routing headers (SEP-2243, J3.2): accept them on the MCP
POSTs and validate consistency with the body so a gateway can route/authorize without parsing
JSON. Then **303** advertise 2026 (flip the default, update the federation card / reference /
Integration; J3.5/J1/J2).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Follows [[Retros/Cluster 300.0]].
Grounded in the `2026-07-28` spec + [[Protocols]] J3.

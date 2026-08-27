# Cluster 302.0 retro — MCP `2026-07-28` routing headers

> Tag **`v302.0.0`**. Phase XXIV (post-gate hardening). MCP `2026-07-28` arc, part 3 (J3.2). No new gate tag.

## What shipped

The SEP-2243 routing headers (`Mcp-Method` / `Mcp-Name`) on the MCP POST transports, so a
gateway can route and authorize a request without parsing its JSON body:

- **`crate::mcp::validate_routing_headers(headers, &request)`** — both headers are optional; when
  present they MUST match the body. `Mcp-Method` must equal `request.method`; `Mcp-Name` must
  equal the request's named target (`params.name` for `tools/call` / `prompts/get`, `params.uri`
  for `resources/read` / `subscribe` / `unsubscribe`). A mismatch is a `400` — a gateway that
  routed/authorized on a header must not be handed a contradicting body.
- Wired into **`POST /mcp`** (single requests — `single_response` now takes `&HeaderMap`) and
  **`POST /mcp/streamable`** (after parse, before dispatch). A **batch** POST skips routing-header
  validation: routing headers describe one op, an array names many.
- An **`Mcp-Name` on a method that names no target** (`tools/list`, `initialize`) is ignored, not
  rejected — the body does no more than the header authorized, so it's not a spoofing risk.

## Surprises / decisions

- **Reject only mismatches, not superfluous headers.** The security property SEP-2243 buys is:
  the body can't lie to a gateway that already routed on the header. That's the mismatch case
  (`Mcp-Method` ≠ method, or `Mcp-Name` ≠ the named target). A stray `Mcp-Name` on `tools/list` is
  harmless (body does less than authorized), so it's tolerated — stricter rejection would only
  add interop friction.
- **Transport-level 400, not a JSON-RPC error.** Consistent with `validate_protocol_version` — a
  contradicting routing header is a transport/gateway fault, surfaced as HTTP `400` before
  dispatch.
- **Batches opt out by design.** A routing header can't describe a heterogeneous batch; validating
  it against the first item (or all) would be wrong, so a batch POST is left to per-item dispatch.

## Capability table extension

`POST /mcp` + `/mcp/streamable` now honor SEP-2243 `Mcp-Method` / `Mcp-Name` routing headers
(optional; mismatch → 400). No new tool/capability.

## Risks identified + still open

- None material. The headers are additive + optional; existing clients that omit them are
  unaffected (all current e2e traffic omits them and still passes).

## Forward look

**303** closes the arc: advertise `2026-07-28` — flip `DEFAULT_PROTOCOL_VERSION` to `2026-07-28`,
update the federation card (`.well-known/maidan.json`), `reference.rs`, and Integration/README
(J3.5), and retire the J2 "temporary honesty" note (J1/J2).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Follows [[Retros/Cluster 301.0]].
Grounded in SEP-2243 + the `2026-07-28` spec + [[Protocols]] J3.

# Cluster 300.0 retro — MCP `2026-07-28` version negotiation

> Tag **`v300.0.0`**. Phase XXIV (post-gate hardening). MCP `2026-07-28` arc, part 1 (J3.1). No new gate tag.

## What shipped

The foundation of the MCP `2026-07-28` upgrade ([[Protocols]] **J3**): the server now
**negotiates** the current revision, additively, without changing the default transport
behavior:

- **`SUPPORTED_PROTOCOL_VERSIONS = ["2026-07-28", "2024-11-05"]`** (`maidan-mcp/src/server.rs`)
  — a current client that requests `2026-07-28` on `initialize` (or via the
  `MCP-Protocol-Version` header, which the HTTP transports validate against this set) now gets
  it echoed. The `2024-11-05` baseline stays fully supported (the doc's one-release fallback).
- **`DEFAULT_PROTOCOL_VERSION` split out from the SUPPORTED set.** `preferred_protocol_version()`
  used to be `SUPPORTED[0]`; it now returns an explicit `DEFAULT_PROTOCOL_VERSION`, held at
  `2024-11-05`. So a version-less or older client keeps the transport it expects, while
  `2026-07-28` is negotiable — the flip of the default to 2026 (and advertising it) is a
  one-liner deferred to the end of the arc, per J3.5 ("advertise 2026 only after 1–4 green").

## Surprises / decisions

- **Additive, not a flip.** The temptation was to make `2026-07-28` the default immediately
  (newest-first `SUPPORTED[0]`). But J3.5 is explicit: don't advertise 2026 until the
  stateless core (J3.3) and routing headers (J3.2) land, or a 2026 client gets `2026-07-28`
  back and then expects behavior that isn't there yet. Decoupling "what we accept" (SUPPORTED,
  newest-first) from "what we default to" (`DEFAULT_PROTOCOL_VERSION`) lets the revision land
  incrementally and safely — a current client that *explicitly* negotiates 2026 works on the
  already-stateless `POST /mcp`, while `/mcp/streamable`'s stateless work is 301's job.
- **No advertisement change yet.** The federation card (`.well-known/maidan.json`), `reference.rs`,
  README/Integration still say `2024-11-05`. Those flip in the arc's final docs cluster, once
  the transport work is green.
- **Every existing test held.** All server e2e version assertions use an explicit `2024-11-05`
  (which still negotiates to itself); the change only *adds* 2026 acceptance.

## Capability table extension

MCP `initialize` + `MCP-Protocol-Version` header now negotiate `2026-07-28` (additively); the
default revision stays `2024-11-05` pending the rest of J3. No new capability/tool.

## Risks identified + still open

- A client that negotiates `2026-07-28` and then uses `/mcp/streamable` still gets the
  `2024-11-05` session model (Mcp-Session-Id) until **301** makes streamable stateless — the
  reason 2026 is not yet the default or advertised.

## Forward look

**301** — stateless streamable core (J3.3–4): no `Mcp-Session-Id` required for a 2026 client,
per-request `MCP-Protocol-Version`, `_meta.io.modelcontextprotocol/clientInfo`; keep live-wait
on `GET /mcp/stream`/WS. Then **302** `Mcp-Method`/`Mcp-Name` routing headers (J3.2), then
**303** advertise 2026 (docs, card, default flip; J3.5/J1/J2).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Grounded in the `2026-07-28` spec
(blog.modelcontextprotocol.io) + [[Protocols]] J3. Opens the MCP upgrade arc.

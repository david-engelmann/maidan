# Cluster 303.0 retro — advertise MCP `2026-07-28` (arc closer)

> Tag **`v303.0.0`**. Phase XXIV (post-gate hardening). MCP `2026-07-28` arc, part 4 / finale (J3.5, J1, J2). No new gate tag.

## What shipped

With negotiation (300), the stateless core (301), and routing headers (302) all green, the
server now **advertises `2026-07-28` as its default** and the docs say so — closing the MCP
upgrade arc (Protocols.md **J3**):

- **`DEFAULT_PROTOCOL_VERSION` flipped to `2026-07-28`** (`maidan-mcp/src/server.rs`) — a
  version-less client now negotiates `2026-07-28`; a client that explicitly requests
  `2024-11-05` still gets it (fallback retained). The 300 negotiation test's default assertion
  updated accordingly.
- **The federation card** (`.well-known/maidan.json`) now reports
  `maidan_mcp::preferred_protocol_version()` instead of a hardcoded `2024-11-05` — stays in
  sync with any future flip automatically.
- **`reference.rs` (the generated MCP reference) + the `maidan-mcp` crate doc** now describe
  `2026-07-28` (stateless Streamable HTTP + SEP-2243 routing headers) with `2024-11-05` as the
  supported fallback.
- **`Integration.md` + `Protocols.md`** advertise `2026-07-28`: the required-upgrade banner,
  the transport table, the streamable how-to, the decision tree, and the J2/J3 gap rows are
  updated to "shipped." The J2 "temporary honesty (say 2024-only)" holding pattern is retired.

## Surprises / decisions

- **Default flip is the advertise switch; the transport keys on the header.** Flipping the
  default changes what a version-less `initialize` *echoes* (`2026-07-28`); the *stateless
  transport* still keys on the explicit `MCP-Protocol-Version: 2026-07-28` header (301). So a
  legacy version-less client is told 2026 but, absent the header, still gets the 2024 SSE
  session — harmless, and exactly "default + docs are 2026" per J3.5. A conformant 2026 client
  sends the header and is fully stateless.
- **`reference.rs` is the template, not the artifact.** `book/src/mcp-reference.md` is
  regenerated from `reference.rs` by `docs.yml` at build (no staleness gate; project practice is
  not to commit the large regen diff — Cluster 280). Edited the template only; CI regenerates
  the published page.
- **Federation card via `preferred_protocol_version()`**, not a second hardcode — one source of
  truth for "what MCP version do we advertise."

## Capability table extension

MCP default is now `2026-07-28` (federation card + reference + docs advertise it); `2024-11-05`
still negotiates on explicit request. **Closes the MCP `2026-07-28` arc (300–303).**

## Risks identified + still open

- A version-less legacy client is echoed `2026-07-28` but, without the header, uses the 2024
  session transport — intentional and harmless (the header is the authoritative transport
  signal). Documented.
- Deferred (niche, logged): stateless server→client (`request_client`) + per-request `_meta`
  clientInfo; `ttlMs`/`cacheScope` on list responses; optional `server/discover`.

## Forward look

MCP arc done. The five-arc program continues: **durable mail retry queue** (Bet 4), then
**Slack / Git projectors** (Bets 1/6), then **public launch**.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Follows [[Retros/Cluster 302.0]].
Closes the arc opened at [[Retros/Cluster 300.0]]. Grounded in the `2026-07-28` spec + [[Protocols]] J3.

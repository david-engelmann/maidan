# Cluster 145.0 retro — MCP conformance basics

> Tag **`v145.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> First of the MCP streamable spec-completeness arc (**145–148**).

## What shipped

- **`initialize` param/version negotiation** — reads the client's
  `protocolVersion` and echoes it if supported, else the preferred version (MCP
  spec §Lifecycle). New API: `SUPPORTED_PROTOCOL_VERSIONS`,
  `is_supported_protocol_version`, `preferred_protocol_version`.
- **`MCP-Protocol-Version` header validation** on `POST /mcp` + `/mcp/streamable`
  (absent OK; unsupported → `400`), via a shared `mcp::validate_protocol_version`.
- **JSON-RPC batching** on `POST /mcp` — array dispatched per element → array of
  responses (quota per request); empty batch → `-32600`.
- **Notifications** (no `id`) → executed, `202` no body (single or batched);
  `notifications/initialized`/`cancelled` accepted, not `MethodNotFound`.

## What was deferred / not covered

| To | What | Why |
|----|------|-----|
| 146 | Batching over the streamable SSE transport | Needs the SSE muxing path; JSON transport is the clean place to land batching first. |
| 148 | Persisting the client's declared `capabilities` | The server→client request layer needs it; 145 negotiates the version but doesn't store client capabilities yet. |
| 146–148 | Adding protocol revisions past `2024-11-05` | Add each as its transport features land, so the supported set never over-claims. |

## Surprises

- **`initialize` had been discarding the client's entire params object** —
  `protocolVersion`, `clientInfo`, and `capabilities` all ignored; the response
  was fully static. The negotiation is the first time the server reads what the
  client sent.
- **Keeping the supported set at `["2024-11-05"]`** is deliberate honesty: the
  streamable transport is a later-revision feature we're *building*, so we don't
  advertise a revision until its transport behavior actually lands.

## Decisions

- **Land JSON-RPC/lifecycle conformance first**, transport features after — they
  need no SSE rework and are cleanly unit/e2e-testable.
- **Batching on the JSON transport (`POST /mcp`) only** this cluster; streamable
  batching pairs with the 146 SSE work.
- **Shared header validator** so both HTTP MCP entrypoints agree.

## Capability table extension

| Capability | Where |
|------------|-------|
| MCP `initialize` version negotiation, `MCP-Protocol-Version` header, JSON-RPC batching + notifications | `maidan-mcp/src/server.rs`, `maidan-server/src/mcp.rs` |

## Risks identified + still open

- **Low.** Additive; single-request-with-id behavior is unchanged, so existing
  clients and the streamable/stdio transports are unaffected (their e2e suites
  pass untouched).

## Forward look

**146** next: `GET /mcp/streamable` server→client SSE stream + `Accept`-header
content negotiation (JSON vs SSE), then resumability (147) and the server→client
request layer (148).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].

# Cluster 412 — Wave 4 #42: an external MCP verifier, and what it found

> Post-gate hardening · target tag `v412.0.0`

## Contract

- Run the official MCP Inspector, built on the official TypeScript SDK, as an
  unmodified external process against a real, authenticated Maidan, in CI.
  Maidan's MCP tests agree with its server by construction. An external client
  is the only test that can disagree.
- Fix what it finds before shipping the verifier, so the job starts green
  rather than as a standing red.

## What the verifier found

1. **The handshake failed.** The SDK (2.0) requests `2025-11-25` and accepts
   `2025-06-18`, `2025-03-26`, `2024-11-05` and `2024-10-07`, but not
   `2026-07-28`. Maidan accepted only `2026-07-28` and `2024-11-05`. The spec
   says a server answers an unsupported request with its latest revision, so the
   SDK got `2026-07-28` back and disconnected before listing a tool. Every
   client built on that SDK was affected.
2. **The session model was the default.** On `/mcp/streamable`, only a request
   carrying `MCP-Protocol-Version: 2026-07-28` was stateless. A 2025 client's
   `initialize` carries no version header, because the header only exists once a
   version is agreed, so it landed in the 2024 SSE-session path. Its follow-ups
   were answered `202` and pushed onto the first stream, which the 2025
   transport forbids.
3. **`resources/list` returned templates.** Each entry's `uri` kept its
   placeholder (`maidan://workspaces/{id}`), which a client reads as a concrete
   resource it can fetch. There was no `resources/templates/list`.
4. **Nullable budget fields used `"type": ["integer", "null"]`.** That's legal
   JSON Schema, but several MCP clients read `type` as a single string and
   reject the tool or drop the constraint. For a spending cap, dropping the
   constraint is silent.

## Decisions

- **Accept every revision since `2024-11-05`; keep `2026-07-28` as the
  default.** From `2025-03-26` on, sessions are optional and the method surface
  is the same. Newer result fields are additive, and clients ignore what they
  don't know.
- **Sessions are opt-in.** Only a `2024-11-05` client gets one: by the version
  its `initialize` negotiates, by its `MCP-Protocol-Version` header, or by
  resuming an open session id. Everything else is stateless. A `2025-03-26`
  client never sends the header, so defaulting to sessions would hand it one it
  has no reason to expect.
- **`resources/list` lists what exists.** That means the caller's own workspace,
  the one resource addressable without naming an id. The id-addressed resources
  are templates.
- **The CI job is report-only.** The behavior is gated by required Rust tests.
  A new Inspector release shouldn't block a merge before it has been triaged.

## Delivery

| Slice | PR | Result |
|-------|----|--------|
| 412.1 | #1010 | `scripts/mcp-inspector.sh` + a report-only `mcp inspector` CI job. All 2025 revisions negotiated and served statelessly, sessions opt-in, `resources/templates/list`, and portable nullable schemas |
| 412.close | close record | [[Retros/Cluster 412]]; recorded as a source record until the maintainer cuts `v412.0.0` |

## Exit criteria

- The Inspector completes `initialize`, `tools/list` (every schema accepted by
  the SDK), `tools/call`, `resources/list` → `resources/read`,
  `resources/templates/list` and `prompts/list` against `/mcp` and
  `/mcp/streamable`, with a real bearer token.
- A required Rust test performs the SDK's handshake for each 2025 revision and
  fails if a session is minted, a notification isn't `202`, or a follow-up opens
  a session.
- No tool schema uses an array-valued `type`.

## Non-goals

- Verifying Cursor, Claude Desktop or other closed clients. The docs no longer
  claim what they speak.
- The A2A TCK (`a2a-tck-ci`), `sdk-interop-oracle`, and `openapi-lint`, the
  other Program B conformance oracles.

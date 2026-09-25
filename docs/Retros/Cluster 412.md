# Cluster 412 retro — an external MCP verifier, and what it found

> Post-gate hardening · source record (no `v412.0.0` tag yet; the tag is the maintainer's) · PR #1010 + close record

## Outcome

The official MCP Inspector, built on the official TypeScript SDK, now runs
unmodified against a real, authenticated Maidan in CI. Before it could run
green, it found four ways a stock client failed against Maidan, and all four are
fixed.

| Slice | PR | Result |
|-------|----|--------|
| 412.1 | #1010 | `scripts/mcp-inspector.sh` plus a report-only `mcp inspector` CI job. Every revision since `2024-11-05` is negotiated; everything but a `2024-11-05` client is served statelessly; `resources/templates/list` exists; nullable schemas use `anyOf`. |

## What the verifier found

1. **The handshake failed.** The SDK asks for `2025-11-25` and accepts the
   2025 and 2024 revisions, but not `2026-07-28`. Maidan accepted only
   `2026-07-28` and `2024-11-05`, so it answered with its latest revision and
   the SDK disconnected before listing a tool. Every client built on that SDK
   was affected.
2. **Sessions were the default.** A 2025 client's `initialize` has no version
   header yet, so it fell into the 2024 SSE-session path, and its follow-ups
   were answered `202` on a stream the 2025 transport forbids.
3. **`resources/list` returned templates.** URIs kept their `{id}`
   placeholders, which a client reads as fetchable resources.
4. **Nullable budget fields used an array-valued `type`.** That's legal JSON
   Schema, but several clients read `type` as a string and drop the constraint,
   which for a spending cap fails silently.

## Decisions

- **Accept every revision since `2024-11-05`; answer `2026-07-28` by default.**
  From `2025-03-26` on, the method surface is the same and newer fields are
  additive.
- **Sessions are opt-in**, granted only to a `2024-11-05` client (by the
  negotiated version, its header, or a resumed session id).
- **`resources/list` lists what exists**, and the id-addressed resources are
  templates.
- **The Inspector job is report-only.** Required Rust tests gate the behaviour;
  a new Inspector release shouldn't block merges before it is triaged.

## What surprised us

- **Maidan's own MCP tests agreed with its server by construction**, and so
  could never have found any of this. An external client was the only test
  that could disagree, and it disagreed on the very first message.
- **The spec's fallback turned a version mismatch into a disconnect.** Replying
  with the server's latest revision is correct, but when that revision is newer
  than anything the client knows, the handshake simply ends.

## Carried forward

- Promote `mcp inspector` to a required check after a clean stretch.
- Wave 4 continues in 413–418 (see [[Open Work]]).

# Cluster 174.0 retro — human-in-the-loop approvals

> Tag **`v174.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Arc 3 (agentic features), part 4 — **the final arc-3 item. Arc 3 is complete.**

## What shipped

- `request_approval` MCP tool: on a streamable session whose client declared the
  `elicitation` capability, it issues a server→client `elicitation/create`, and
  maps the human's `{action: accept|decline|cancel, content?}` to
  `{approved, action, content}`. A gate an agent can `await` before a sensitive
  action.
- New `tools/approval.rs`, wired into dispatch + `workspace:read` capability +
  catalog + both MCP contracts.

## What was deferred

Persisted/auditable approvals (a pending-approval object resolved async via
REST/UI); approval policies (which actions require approval); multi-approver
quorum. All build on this synchronous primitive.

## Surprises

- **Almost entirely a mirror of `summarize_thread`.** The whole server→client
  transport (capability gating, GET-stream delivery, response correlation,
  timeout) was already built and battle-tested across 148/154/155. Adding a
  *second* organic caller was a small tool + the exact same wiring — the
  transport investment paid off twice now (sampling + elicitation).
- **JSON-in-JSON in the test assertion.** The tool result is a text content
  block whose text is serialized approval JSON, so `body.contains("\"approved\":true")`
  failed on the *escaped* quotes — the substring check had to become a two-layer
  parse. The tool was correct on the first run; only the assertion was naive.

## Decisions

- **Reuse the elicitation transport, no persistence (v1).** Synchronous over the
  session like `summarize_thread`; `workspace:read` (elicits, mutates nothing);
  not in the channel-access gate (it's a session primitive, not channel content).
- **`accept` ⇒ approved.** The elicitation `action` is the decision; a
  timeout/absent GET stream fails closed (not approved).

## Capability table extension

| Change | Where |
|--------|-------|
| MCP `request_approval` HITL gate via `elicitation/create` | `maidan-mcp/src/tools/approval.rs` |

## Risks identified + still open

- **Low.** Additive tool; reuses a proven transport; no state mutation. The only
  behavioral edge is fail-closed on timeout, which is the safe default for an
  approval gate.

## Forward look

**Arc 3 (agentic features) is complete** — task assignment/handoff (171),
structured backpressure (172), structured message content (173), HITL approvals
(174). Next is **arc 4 — token round 3**: MCP `search_messages` `snippet_only`
parity, capability-filtered `tools/list` + trimmed catalog descriptions, lean
write-acks / omit-empty metadata, opt-in lean event frames.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].

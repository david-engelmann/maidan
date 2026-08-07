# Cluster 174.0 — agentic: human-in-the-loop approvals

**Theme:** Arc 3 (agentic features), part 4 (final) — a HITL approval gate over
the MCP elicitation transport.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v174.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| MCP `request_approval` tool → server→client `elicitation/create` → `{approved, action, content}` | `maidan-mcp/src/tools/approval.rs` (new), `tools/mod.rs`, `catalog.rs` |
| Contracts | `contracts/mcp-tool-names.json`, `contracts/mcp-capability-map.json` |

## Why

An agent doing something sensitive should be able to **ask the human** first.
The MCP server→client transport (`request_client`, Cluster 148; GET-stream
delivery, 154) already carries `elicitation/create` — the spec's mechanism for
the server to prompt the human and collect a structured response. `summarize_thread`
(155) proved this path with `sampling/createMessage`; `request_approval` is its
elicitation analogue: a gate an agent can `await` before proceeding.

## How

`request_approval(prompt, schema?)` (on a streamable session whose client
declared the `elicitation` capability) issues `elicitation/create` with the
`prompt` as `message` and an optional `requestedSchema`. The client presents it
to the human, who returns `{action: accept|decline|cancel, content?}`. The tool
maps that to `{approved: action=="accept", action, content}`. A timeout or a
missing GET stream surfaces as an error (not approved).

## Key decisions

- **Reuse the elicitation transport; no persistence.** v1 is synchronous over
  the session, exactly like `summarize_thread` — no new table, no async
  operator queue. `workspace:read` capability (it elicits, mutates nothing).
- **`accept` ⇒ approved.** The elicitation `action` *is* the decision; the
  optional `content`/`requestedSchema` carry structured detail (e.g. a reason).
- **No channel-access gate.** It's a session-level HITL primitive, not
  channel content, so it's not in the `enforce_channel_access` pre-dispatch set.

## Non-goals (deferred)

Persisted/auditable approvals (a pending-approval object an operator resolves
async via REST/UI); approval policies (which actions require approval);
multi-approver quorum. All would build on this primitive.

## Exit criteria

- An MCP client with `elicitation` gets an approval prompt over the GET stream
  and its `accept` resolves the tool as `approved:true`; suites green — **met**.
- `v174.0.0` tagged. **Arc 3 (agentic features) complete.**

## Verification & limits

- `request_approval_tool_elicits_the_human_via_the_client` (mcp_streamable_e2e):
  open session declaring `elicitation` → GET stream → `tools/call
  request_approval` → the `elicitation/create` request arrives on the GET stream
  with the prompt → POST `{action:accept}` → the tool resolves `approved:true`
  (+ `action`, passthrough `content`). MCP capability-matrix + contract tests
  confirm catalog↔contract sync + `workspace:read` gating.
- Limit: synchronous only — if no human/GET stream answers within the client-
  request timeout, it errors (fail-closed: not approved).

## References

- [[Retros/Cluster 174.0]]; `maidan-mcp/src/tools/approval.rs`,
  `server.rs::request_client`. Program: [[Roadmap]] + memory
  `maidan-next-arc-program`.

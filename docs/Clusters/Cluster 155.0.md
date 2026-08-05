# Cluster 155.0 — sampling-backed `summarize_thread` (first `request_client` caller)

**Theme:** Lane 3 (of the three-lane plan), part 2 — give `request_client` a
real in-tree caller and close the plan.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v155.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| Thread an optional streamable session id through tool dispatch | `crates/maidan-mcp/src/server.rs` (`handle_in_session`, `dispatch`, `tools_call`) |
| Pass the session id from the transport | `crates/maidan-server/src/mcp_streamable.rs` (JSON-accept POST + both SSE paths) |
| `summarize_thread` tool (gather thread → `request_client` sampling → return) | `crates/maidan-mcp/src/tools/thread.rs` + `tools/mod.rs` + `catalog.rs` + both `contracts/mcp-*.json` |

## Why

`request_client` (148) + its GET-stream delivery (154) were transport capability
with **no organic caller**. `summarize_thread` is one: an agent calls the tool,
the server gathers the thread transcript and asks the *connected client* to
sample a summary (`sampling/createMessage`) over the canonical GET stream — the
server never holds an API key. This also required threading the streamable
session id into tool dispatch, which nothing did before.

## Non-goals

- HITL approvals / elicitation-confirm on destructive tools — a separate feature
  (now scheduled in the post-v155 agentic-features arc).
- Any change to the 154 delivery mechanism.

## PR ladder (actual)

| # | Title |
|---|--------|
| 155.0.1 | `feat(mcp): sampling-backed summarize_thread — first request_client caller` (#400) |
| 155.0.retro | `docs(retro): Cluster 155.0 + v155.0.0 tag prep` |

## Exit criteria

- `summarize_thread` round-trips a sampling request over the GET stream and
  returns the client's completion; session id reaches the tool; tests green —
  **met**.
- `v155.0.0` tagged after retro; three-lane plan closed.

## Verification & limits

- E2E `summarize_thread_tool_samples_via_the_client`: open session + GET stream,
  call the tool (spawned; blocks on the client), read the sampling request off
  the GET stream, POST the summary, assert the tool result carries it. Catalog +
  capability-map contract tests green with the new tool.
- Limit: needs a client that supports sampling and an open GET stream; otherwise
  the tool returns a clear error (no hang).

## References

- [[Retros/Cluster 155.0]]; [[Clusters/Cluster 154.0]]; `tools/thread.rs`,
  `server.rs` (`handle_in_session`), `mcp_streamable.rs`. Post-v155 program:
  see [[Roadmap]] + memory `maidan-next-arc-program`.

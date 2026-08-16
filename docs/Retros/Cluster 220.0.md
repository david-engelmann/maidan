# Cluster 220.0 retro — the DAG reaches agents, and the one-id gate needs a hand

> Tag **`v220.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Program B (agentic orchestration), part 4.

## What shipped

- MCP tools `add_thread_dependency` + `list_thread_dependencies`, with the full
  5-place wiring. Agents can now build and inspect the task DAG over MCP, not just
  respect it.

## Surprises / decisions

- **The pre-dispatch gate only guards one id.** `enforce_channel_access` resolves a
  single arg (`thread_id` / `channel_id` / `message_id`) and checks access before
  the handler runs — perfect for `list_thread_dependencies`. But
  `add_thread_dependency` touches *two* threads, and the gate only covers the
  primary `thread_id`. So the handler carries `auth` and does its own
  `ensure_thread_access` on `depends_on` plus the same-workspace check — the MCP
  analogue of the REST route's both-thread RBAC. A tool that references a second
  entity can't lean entirely on the gate.
- **Router errors don't convert to `McpError`.** `resolve_thread_context` returns a
  router error with no `From` into `McpError`, so `?` doesn't compile — the existing
  MCP handlers `.map_err(|e| McpError::InvalidParams(e.to_string()))`. A small
  papercut worth remembering: not every `await?` that works in a REST handler works
  in an MCP one.
- **Five places, sorted, or a contract test reds.** The tool needed: handler,
  `dispatch` arm, `required_capability` arm, the gate's `thread_id` list,
  `catalog.rs` schema, and *both* `contracts/mcp-*.json` entries in sorted order.
  `tools_catalog_contract` + `mcp_capability_map_contract` enforce the sync, so
  missing or mis-sorted entries fail fast — the checklist is load-bearing.
- **Scope: add + list, not the whole REST surface.** Remove and dependents-listing
  stay REST-only; add/list are what an agent building a sub-task graph needs, and
  keeping the tool count tight keeps the catalog lean (the capability-filtered
  `tools/list` from 176 still shows only what a token can invoke).

## Capability table extension

| Change | Where |
|--------|-------|
| MCP DAG tools (`add_thread_dependency`, `list_thread_dependencies`) | `tools/thread.rs`, `tools/mod.rs`, `tools/catalog.rs`, `contracts/mcp-*.json` |

## Risks identified + still open

- **The DAG's core surface is complete** over REST + MCP (build, inspect,
  respect-on-claim). Transitive cycle prevention + a "task ready" event remain later
  items; remove/dependents over MCP can follow if agents need them.

## Forward look

With the DAG surfaced end-to-end, Program B moves to its next lanes: scheduled/
recurring tasks, a capability registry + skill routing (match work to agents),
queue-depth metrics, and coordination waits + structured results. Then Programs C
(notifications & reach) and D (scale & durability).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues [[Retros/Cluster 219.0]].

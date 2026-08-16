# Cluster 220.0 — task-dependency DAG: MCP tools

**Theme:** Program B (agentic orchestration), part 4 — the MCP tools for building
and inspecting the task DAG, so agents (not just REST callers) can wire it.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v220.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| MCP `add_thread_dependency` + `list_thread_dependencies` (handlers + 5-place wiring) | `tools/thread.rs`, `tools/mod.rs`, `tools/catalog.rs`, `contracts/mcp-tool-names.json`, `contracts/mcp-capability-map.json` |

## Why

Cluster 219 gave the DAG a REST management surface. Agents drive Maidan over MCP,
so this adds the equivalent MCP tools — the parallel to how assignment shipped REST
(190) then MCP (191). Agents can already *respect* the DAG (readiness-aware
`claim_next`, 218) and now can *build* it.

## The change

- **`add_thread_dependency`** (`thread_id`, `depends_on_thread_id`;
  `thread:transition`) — the primary `thread_id`'s channel access is enforced by the
  pre-dispatch gate (`enforce_channel_access`'s `thread_id` arm); the handler adds an
  `ensure_thread_access` on the **`depends_on`** thread (the gate only resolves one
  id) plus a same-workspace guard. Idempotent; self-dependency rejected.
- **`list_thread_dependencies`** (`thread_id`; `workspace:read`) — returns
  `{ dependencies, ready }`; the `thread_id` arm of the gate covers access.

Full MCP 5-place wiring: handlers, `dispatch` arms, `required_capability` arms, the
pre-dispatch gate's `thread_id` list, `catalog.rs` schemas, and both
`contracts/mcp-{tool-names,capability-map}.json` (sorted).

## Exit criteria

- An MCP agent can add + list task dependencies (with readiness) — **met**.
- `v220.0.0` tagged.

## Verification & limits

- `tools_catalog_contract` + `mcp_capability_map_contract` (catalog ↔ contracts
  sync) + `mcp_capability_matrix_e2e` (each tool denies the wrong capability) green.
- Store behaviour is already proven (217/218 store suites; 219 REST e2e).
- **Limits:** dependents-listing + remove stay REST-only for now (add/list are the
  agent-facing essentials). Transitive cycle prevention + a "task ready" event remain
  later items. With 220, the DAG's read/write surface is complete over REST + MCP.

## References

- [[Retros/Cluster 220.0]]; `tools/thread.rs`. Program B: [[Roadmap]] + memory
  `maidan-next-arc-program`. Continues [[Retros/Cluster 219.0]].

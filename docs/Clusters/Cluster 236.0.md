# Cluster 236.0 — Arc F MCP + `wait_for_result` (closes Program B)

> Program B (agentic orchestration), part 20 — **closes Arc F and Program B**.
> Phase XXIV post-gate hardening. Tag **`v236.0.0`**. No new gate tag.

## Goal

Give MCP-native agents the structured-result surface the REST side got in 235, and
add the **coordination wait** that makes the "spawn sub-tasks, wait, aggregate their
outputs" pattern a single-call primitive. This is the last piece of Program B.

## Scope

| Change | Where |
|--------|-------|
| MCP `set_thread_result` (`thread:transition`) + `get_thread_result` (`workspace:read`) — the twins of 235's REST | `tools/thread.rs`, `tools/mod.rs`, `tools/catalog.rs`, `contracts/mcp-*.json` |
| MCP `wait_for_result` (`workspace:read`) — block on a thread's `ThreadResultSet`, return the result payload | `tools/thread.rs`, `tools/mod.rs`, `tools/catalog.rs`, `contracts/mcp-*.json` |
| MCP `get_dependency_results` (`workspace:read`) — a parent aggregates its dependencies' results | `tools/thread.rs`, `tools/mod.rs`, `tools/catalog.rs`, `contracts/mcp-*.json` |

## Design decisions

- **`wait_for_result` returns the payload, not the pointer event.** `wait_for_ready`
  (223) returns the `ThreadReady` event because readiness *is* the signal. A result's
  value is the payload, so `wait_for_result` fetches `get_thread_result` after the
  `ThreadResultSet` fires and returns *that*. The event filter pins `thread_id`, so
  any frame that arrives is the one being awaited; the pre-dispatch gate already
  enforced access, so there's no per-event RBAC re-check (unlike `wait_for_ready`,
  whose workspace/channel-scoped filter spans threads).
- **`get_dependency_results` projects the raw payload.** Each entry is
  `{thread_id, result}` where `result` is the dependency's raw JSON output (or `null`
  if not produced yet) — not the full `ThreadResult` envelope, which would nest a
  redundant `thread_id`. A parent wants each child's *output*; provenance
  (`produced_by`/`produced_at`) is a `get_thread_result` away. `null` cleanly marks a
  pending dependency.
- **All four key on `thread_id`**, so they join the existing pre-dispatch
  `ensure_thread_access` gate arm. `get_dependency_results` gates the *parent* thread;
  its dependencies (which may live in other channels) are filtered in-handler by
  `can_access_thread`, like `list_assigned_threads`.
- **`produced_by = auth.member_id`** (a NOT-NULL FK), so `set_thread_result`'s test
  uses a real session member (`AuthContext::from_session`), not the bypass nil member.

## Non-goals

- No new store logic — REST (235) + MCP read/write the identical `set/get_thread_result`
  methods (Cluster 234). No new event kind — `ThreadResultSet` (235) is reused.

## Risks

- MCP tool wiring is the 5-place drill + both sorted contracts; the contract-sync
  tests (`tools_catalog_contract`, `mcp_capability_map_contract`) catch a miss.

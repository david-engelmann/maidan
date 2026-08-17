# Cluster 223.0 — `wait_for_ready`: block until a task is claimable

> Program B (agentic orchestration), part 7. Phase XXIV post-gate hardening.
> Tag **`v223.0.0`**. No new gate tag.

## Goal

Give an MCP agent a blocking "await work" primitive for the DAG. Cluster 222
started pushing readiness (`ThreadReady`), but the only way to consume it was the
generic `GET /mcp/stream` subscribe. `wait_for_ready` is the `wait_for_mention`
(Cluster 196) analogue: a single long-poll tool call that parks until a task
becomes ready, then returns it.

## Scope

| Change | Where |
|--------|-------|
| MCP `wait_for_ready` (`workspace:read`): subscribe to `ThreadReady`, return the first accessible ready thread or `null` on timeout; optional `channel_id` scope; per-event `can_access_thread` filter | `tools/thread.rs` |
| 5-place wiring: dispatch, capability, channel gate (optional `channel_id`), catalog schema, both `contracts/mcp-*.json` | `tools/mod.rs`, `tools/catalog.rs`, `contracts/mcp-*.json` |

## Design decisions

- **Mirror `wait_for_mention` exactly.** Same `timeout_ms` default/clamp
  (30 s / 1 ms–300 s), same `timeout_at` loop, same lag-marker skip, same
  RBAC-per-event filter, same live-only semantics + "drain first" guidance. One
  proven shape, two events.
- **Scope: optional `channel_id`, else workspace.** A `channel_id` gates
  pre-dispatch (a caller can't long-poll a private channel they're not in);
  omitting it awaits any thread in `auth.workspace_id`, with per-event
  `can_access_thread` dropping inaccessible private-channel readiness. Fits
  `ThreadReady`'s shape (a ready task may be unassigned — it has no member axis to
  filter on, unlike a mention).
- **Handler in `tools/thread.rs`, not `member.rs`.** It's a DAG/thread primitive;
  `wait_for_mention` stays the member analogue.

## Non-goals / deferred

- Returning *already-ready* work (the tool is live-only by design; `claim_next` /
  `list_assigned_threads` cover the backlog, and `at_least_once` SSE the resumable
  path).
- A REST equivalent (WS/SSE already stream `thread_ready`).

## Risks

- **Live-only misuse** — an agent that calls `wait_for_ready` without first draining
  ready work can miss a task that became ready just before subscribing. Documented
  in the tool description + this plan; the resumable SSE path is the mitigation.

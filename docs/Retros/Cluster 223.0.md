# Cluster 223.0 retro — the DAG gets a doorbell

> Tag **`v223.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Program B (agentic orchestration), part 7.

## What shipped

- `wait_for_ready` — an MCP long-poll that blocks until a task becomes claimable
  (its last blocking dependency reaches a terminal state → `ThreadReady`, Cluster
  222), returning the ready thread or `null` on timeout. The `wait_for_mention`
  analogue for the DAG.

## Surprises / decisions

- **222 built the wire; 223 is the phone.** Cluster 222's `ThreadReady` already
  flowed over the generic subscribe, so this cluster is pure ergonomics — one tool
  call instead of "open an SSE stream and filter." Small, but it's the difference
  between an agent *reacting* to readiness and an agent *polling* for it.
- **The template did the heavy lifting.** `wait_for_mention` (196) had already
  solved every hard part — the `timeout_at` park, the lag-marker skip, the
  per-event RBAC filter, the "live-only, drain first" contract, the `join!`-based
  test that dodges the subscribe race. Copying its shape (with `ThreadReady` +
  optional `channel_id` instead of `member_id`) meant the risk was near-zero and
  the reviewer sees a familiar pattern.
- **`channel_id` is optional, and that changed the gate.** Unlike the other
  channel-gated tools, `wait_for_ready`'s `channel_id` may be absent (await the
  whole workspace). The pre-dispatch `enforce_channel_access` arm already guards
  only *when the field is present*, so adding `wait_for_ready` to the
  `list_threads | claim_next_thread` arm was exactly right — no new gate logic.
- **No member axis.** A ready task is often *unassigned* (that's the point — it's
  waiting to be claimed), so unlike a mention there's nothing to filter by member.
  Workspace + optional channel is the natural scope.

## Capability table extension

| Change | Where |
|--------|-------|
| MCP `wait_for_ready` long-poll over `ThreadReady` | `tools/thread.rs`, `tools/mod.rs`, `tools/catalog.rs`, `contracts/mcp-*.json` |

## Risks identified + still open

- **Live-only** — misuse (not draining ready work first) can miss a just-ready
  task; documented, with the `at_least_once` SSE path as the resumable mitigation.

## Forward look

The DAG surface is now complete end-to-end: build (`add_thread_dependency`),
inspect (`list_thread_dependencies`), enforce acyclicity (221), claim
readiness-aware (218), push readiness (222 `ThreadReady`), and block on it (223
`wait_for_ready`). Program B moves on: scheduled/recurring tasks, a capability
registry + skill routing, queue-depth metrics, and coordination waits + structured
results. Then Programs C (notifications & reach) and D (scale & durability).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues [[Retros/Cluster 222.0]].

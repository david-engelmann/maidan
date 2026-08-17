# Cluster 229.0 retro — the scheduler reaches agents

> Tag **`v229.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Program B (agentic orchestration), part 13.

## What shipped

- MCP `create_task_schedule` + `list_task_schedules` — an agent schedules its own
  work and lists what's scheduled. With this the scheduler subsystem is complete
  end to end: store (226) → worker (227) → REST (228) → MCP (229).

## Surprises / decisions

- **The nil-member trap, again — and the same fix.** Like the REST cluster,
  `create_task_schedule` stamps `created_by = auth.member_id`, so the MCP test can't
  use the bypass context (nil member → FK violation). The `wait_for_mention` /
  `wait_for_ready` tests already had the pattern: build a real-member session with
  `AuthContext::from_session`. Reusing it gave a test that exercises real capability
  + channel-access gating instead of skipping it.
- **Gate one, filter the other.** `create` has a `channel_id`, so it slots into the
  pre-dispatch channel gate for free. `list` is a workspace-scoped aggregate with no
  id to gate, so it filters its results by `can_access_channel` in the handler —
  exactly the `list_assigned_threads` shape. Two tools, two different (established)
  RBAC mechanisms.
- **Two tools, not four.** Delete and pause-resume are operator actions that REST
  (228) and a future UI cover; an agent's need is to *create* and *see* its
  schedules. Shipping the minimal useful pair keeps the catalog lean — the same call
  the DAG MCP cluster (220) made with add+list.

## Capability table extension

| Change | Where |
|--------|-------|
| MCP `create_task_schedule` + `list_task_schedules` | `tools/schedule.rs`, `tools/mod.rs`, `tools/catalog.rs`, `contracts/mcp-*.json` |

## Risks identified + still open

- None new — the scheduler's correctness (atomic claim, at-most-once firing) lives
  in the store/worker (226/227), untouched here.

## Forward look

The **scheduled/recurring-task subsystem is complete** (226–229). Program B moves to
its last lanes: a capability registry + skill routing (match work to agents by
declared skill), then coordination waits + structured results. Then Programs C
(notifications & reach) and D (scale & durability).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues [[Retros/Cluster 228.0]].

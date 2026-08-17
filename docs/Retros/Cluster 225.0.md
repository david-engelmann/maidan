# Cluster 225.0 retro — queue-depth reaches MCP

> Tag **`v225.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Program B (agentic orchestration), part 9.

## What shipped

- `get_queue_depth` MCP tool — `{channel_id}` → `{open, ready, assigned, blocked}`,
  the MCP twin of Cluster 224's REST endpoint, over the same
  `Store::channel_queue_depth`.

## Surprises / decisions

- **Nothing new to get wrong.** By the 224 → 225 split, the substantive work
  (the aggregate query, both backends, the claimability predicate) already shipped
  and is tested; this cluster is transport only. The tool body is one store call
  wrapped in `content_json`, so REST and MCP are guaranteed to report identical
  numbers — the value of splitting *after* the store method exists.
- **The gate arm was already the right shape.** `channel_id` is required here (no
  optional-scope subtlety like `wait_for_ready`), so it joined the plain
  `channel_id` pre-dispatch arm and inherited private-channel access enforcement
  for free.
- **Five sorted places, contract-checked.** Handler, dispatch, capability, gate,
  catalog, and both `contracts/mcp-*.json` — `tools_catalog_contract` +
  `mcp_capability_map_contract` fail fast on a missing/mis-sorted entry, so the
  checklist is self-verifying.

## Capability table extension

| Change | Where |
|--------|-------|
| MCP `get_queue_depth` over `channel_queue_depth` | `tools/thread.rs`, `tools/mod.rs`, `tools/catalog.rs`, `contracts/mcp-*.json` |

## Risks identified + still open

- None new — inherits 224's point-in-time-snapshot caveat; adds no logic.

## Forward look

The task-queue subsystem (217–225) is now feature-complete over REST + MCP: build /
inspect / acyclicity / readiness-aware claim / reactive push / blocking wait / depth
read. Program B turns to its remaining lanes: scheduled/recurring tasks, a capability
registry + skill routing, and coordination waits + structured results. Then Programs
C (notifications & reach) and D (scale & durability).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues [[Retros/Cluster 224.0]].

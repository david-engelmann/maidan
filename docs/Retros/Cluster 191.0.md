# Cluster 191.0 retro — the work queue reaches MCP agents

> Tag **`v191.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Arc C (agentic task-queue depth), part 2.

## What shipped

- MCP tools `claim_next_thread` (channel-gated pre-dispatch) and
  `list_assigned_threads` (member-scoped, in-handler RBAC filter) — the deferred
  half of Cluster 190 — wired through `dispatch`, `required_capability`,
  `catalog`, and both contract files.

## Surprises / decisions

- **The two tools sit on opposite sides of the RBAC seam, by design.**
  `claim_next_thread` carries a `channel_id`, so it drops cleanly into the
  existing pre-dispatch `enforce_channel_access` gate (one line, alongside
  `list_threads`) — no filtering in the handler. `list_assigned_threads` carries
  only a `member_id`, which the gate can't resolve to a channel, so it *must*
  filter its own result like the other aggregate reads (`search_messages`,
  `list_channels`). Recognizing that split up front is exactly why 190 deferred
  this — the naive "just add two tools" would have left `list_assigned` unfiltered.
- **Contract-sync tests are a good backstop.** Adding an MCP tool touches five
  places (handler, dispatch, capability, catalog, two contracts); the
  `mcp_tool_names` / `mcp_capability_map` contract tests fail loudly if any drift,
  which caught nothing this time only because I did all five — but that's the
  point.

## Capability table extension

| Change | Where |
|--------|-------|
| MCP `list_assigned_threads` + `claim_next_thread` (RBAC-filtered / channel-gated) | `maidan-mcp/src/tools/` |

## Risks identified + still open

- **Net additive, RBAC-consistent** — the member-scoped list filters by caller
  access exactly like the REST route and `search_messages`. Open (Open Work /
  next cluster): no claim **lease** (a claimed-then-dead agent holds the thread);
  `claim_next` channel-scoped only.

## Forward look

Arc C continues: **claim leases + reclaim** (dead-agent recovery — the natural
next step now that pull-based claiming exists), `roots/list` tool, structured
tool-call transcripts, `wait_for_mention`, handoff notes, federation
`parts→content`.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Completes
[[Retros/Cluster 190.0]]'s deferral.

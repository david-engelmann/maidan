# Cluster 191.0 — agentic: MCP tools for the assignment read-side

**Theme:** Arc C (agentic task-queue depth), part 2 — surface Cluster 190's
assignment read-side to MCP-native agents (the deferred half of 190).

**Ladder:** Post-gate — **Phase XXIV**, tag **`v191.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| MCP tools `list_assigned_threads` + `claim_next_thread` | `maidan-mcp/src/tools/thread.rs`, `mod.rs`, `catalog.rs` |
| Contracts + capability wiring | `contracts/mcp-tool-names.json`, `contracts/mcp-capability-map.json` |

## Why

Cluster 190 shipped the assignment read-side over REST but deferred the MCP tools
because the member-scoped `list_assigned` doesn't fit the pre-dispatch
channel-access gate. Agents drive Maidan through MCP, so the tools are what make
the work queue actually usable for an agent.

## The fix

- **`claim_next_thread`** (`channel_id`, `member_id`) — atomically claim the
  oldest unassigned thread in a channel. Channel access is enforced **pre-dispatch**
  (added to the `enforce_channel_access` `channel_id` arm, alongside
  `list_threads`), so the tool handler stays a thin store call. Capability
  `thread:transition`.
- **`list_assigned_threads`** (`member_id`) — the member's queue. A member-scoped
  **aggregate** read that the channel gate can't cover, so the handler filters the
  result to threads the *caller* can access via `can_access_thread` — the same
  pattern `search_messages` uses (Cluster 162). Capability `workspace:read`.

Both are registered in `required_capability`, `dispatch`, `catalog`, and both
contract files (kept sorted; the contract-sync tests enforce it).

## Exit criteria

- An MCP agent can list its assigned threads and claim the next one; the
  member-scoped list is RBAC-filtered to the caller's access — **met**.
- `v191.0.0` tagged.

## Verification & limits

- `maidan-mcp` inline `mcp_assignment_read_side_claims_lists_and_filters`:
  `claim_next_thread` takes the oldest then `null`; `list_assigned_threads` shows
  a public-channel assignment but **filters** a private-channel one the caller
  isn't a member of. The `mcp_tool_names` + `mcp_capability_map` contract-sync
  tests stay green.
- Limits: still no claim **lease** (a claimed-then-dead agent holds the thread —
  the next Arc-C cluster); `claim_next` is channel-scoped.

## References

- [[Retros/Cluster 191.0]]; `maidan-mcp/src/tools/thread.rs`. Program:
  [[Roadmap]] + memory `maidan-next-arc-program` (Arc C). Completes
  [[Retros/Cluster 190.0]]'s deferral.

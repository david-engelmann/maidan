# Cluster 225.0 — `get_queue_depth` MCP tool

> Program B (agentic orchestration), part 9. Phase XXIV post-gate hardening.
> Tag **`v225.0.0`**. No new gate tag.

## Goal

Give an MCP-only orchestrator the queue-depth read that REST got in Cluster 224 —
the second half of the standard REST/MCP split (219 REST → 220 MCP; 224 REST → 225
MCP).

## Scope

| Change | Where |
|--------|-------|
| MCP `get_queue_depth` (`workspace:read`): `{channel_id}` → `{open, ready, assigned, blocked}` over `Store::channel_queue_depth` | `tools/thread.rs` |
| 5-place wiring: dispatch, capability, channel gate (`channel_id`), catalog schema, both `contracts/mcp-*.json` (sorted) | `tools/mod.rs`, `tools/catalog.rs`, `contracts/mcp-*.json` |

## Design decisions

- **Thin twin.** The whole tool is `channel_queue_depth` (the Cluster-224 store
  method, tested both backends) wrapped in `content_json`. No new store logic —
  REST and MCP read the exact same aggregate, so the counts can never diverge.
- **Channel-gated pre-dispatch.** `channel_id` is required, so it slots into the
  existing `list_threads | claim_next_thread | wait_for_ready | get_queue_depth`
  gate arm — a caller can't read a private channel's depth without access.

## Non-goals / deferred

- Workspace-wide roll-up (same as 224 — a channel is the natural queue).

## Risks

- None beyond 224's (point-in-time snapshot); this cluster adds no new logic, only
  transport.

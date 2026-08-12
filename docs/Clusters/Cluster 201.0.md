# Cluster 201.0 — workspace-sharded event fan-out

**Theme:** Arc D (performance & scale), part 4 — stop waking every subscriber for
every event. Route a publish only to the subscribers that could match it.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v201.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| `ShardedBroadcast` — per-workspace shards + a global shard; publish routes to the event's workspace shard + global; subscribe picks by `filter.workspace_id`; lazy create + prune-on-drop | `maidan-bus/src/sharded.rs` |
| `InMemoryBus` uses a `ShardedBroadcast` | `maidan-bus/src/inmem.rs` |
| `PostgresBus` local broadcast (LISTEN task + polled publish + subscribe) uses a `ShardedBroadcast` | `maidan-bus/src/postgres.rs` |

## Why

The event bus used a single `tokio::broadcast` channel. Every publish went to
*every* subscriber, which then ran `EventFilter::matches` and discarded the
events for other workspaces. So in a busy multi-tenant deployment, a workspace-A
event woke all of workspace B/C/D…'s subscribers just to be filtered away —
O(total subscribers) work per event, independent of relevance. That's the
fan-out cost that doesn't scale.

## The change

`ShardedBroadcast` keeps a `global` channel plus a `HashMap<WorkspaceId,
Sender>` of workspace shards:

- **publish**: send to `global`; if the event is workspace-scoped *and* that
  workspace has a live shard, send to it too (one clone — a `send` moves). The
  map lock is held only for an O(1) lookup.
- **subscribe**: if the filter pins a workspace, get-or-create that shard and
  return a receiver; else return a `global` receiver. Shards are created and
  subscribed **under the map lock**, so a concurrent prune can't drop a shard
  that just gained this receiver. Dead shards (0 receivers) are pruned here —
  subscribe is far rarer than publish, so the scan stays off the hot path.

A workspace-scoped subscriber now reads only its workspace's shard; a global
subscriber (operator, or any filter without a `workspace_id`) reads everything.
This is an **optimization under the existing `EventFilter`** — the filter still
narrows by channel/thread/kind, just on far fewer events. Behavior is unchanged
(a workspace-scoped filter never matched another workspace's events), so the win
is pure efficiency. Presence and resource-notify ride separate channels and are
untouched.

## Exit criteria

- A publish reaches only the relevant workspace's + global subscribers; delivery
  is unchanged — **met**.
- `v201.0.0` tagged.

## Verification & limits

- `sharded::tests` (unit): a workspace-A subscriber sees A's event and not
  another workspace's; a global subscriber sees every workspace; a dead shard is
  pruned on the next subscribe.
- All server delivery e2es green — `ws_subscribe_e2e`, `event_emission_e2e`,
  `subscribe_grants_e2e`, `mcp_stream_at_least_once_e2e`, `mcp_streamable_e2e`,
  `ui_ws_tail_e2e`, `two_replica_durable_state_e2e`, `presence_ws_e2e` — plus the
  `maidan-bus` Postgres LISTEN/NOTIFY suite (exercises the sharded local
  broadcast against a real Postgres).
- Limit: shards are keyed by workspace; a subscriber with no `workspace_id`
  filter falls back to the global shard (no sharding benefit — correct, just not
  optimized). The clone-per-dual-send and the prune scan are the only added
  costs, both bounded.

## References

- [[Retros/Cluster 201.0]]; `maidan-bus/src/sharded.rs`. Program: [[Roadmap]] +
  memory `maidan-next-arc-program` (Arc D). Batched `pg_notify` + read-replica
  routing were assessed and deferred — see [[Open Work]].

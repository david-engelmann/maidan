# Cluster 224.0 — channel task-queue depth

> Program B (agentic orchestration), part 8. Phase XXIV post-gate hardening.
> Tag **`v224.0.0`**. No new gate tag.

## Goal

Give an orchestrator visibility into a channel's work queue: how many tasks are
claimable now, how many are in progress, how many are blocked on dependencies —
the signal for deciding whether to spin up more agents. The task-queue subsystem
(217–223) had every *actor-facing* primitive but no *aggregate read*.

## Scope

| Change | Where |
|--------|-------|
| `QueueDepth { open, ready, assigned, blocked }` type | `maidan-types/src/models.rs` |
| `Store::channel_queue_depth(channel_id)` — one aggregate query, both backends | `store.rs`, `store/{sqlite,postgres}/threads.rs`, `store/{sqlite,postgres}/mod.rs` |
| REST `GET /channels/:cid/queue-depth` (`workspace:read` + channel access) | `routes/channel.rs`, `app.rs` |
| New-route preflight: OpenAPI path + `QueueDepth` schema reg, `http-capability-map` GET entry | `openapi/paths/api.rs`, `openapi/mod.rs`, `contracts/http-capability-map.json` |

## Design decisions

- **`ready` = the `claim_next` predicate, verbatim.** The `ready` count reuses the
  exact SQL clause `claim_next` uses to pick work (unassigned OR lease-expired, AND
  every dependency terminal), so the number an orchestrator sees is precisely what a
  worker would be able to claim. `assigned` is its live-lease complement; `blocked`
  is the not-claimable-because-of-deps remainder. The three partition `open`.
- **On-demand DB aggregate, not a Prometheus label.** A per-channel gauge would blow
  up metric cardinality; a single scalar-aggregate query on demand is the exact call
  Cluster 188 (`workspace_usage`) made for the same reason, and it gives exact
  counts rather than sampled rates.
- **Postgres `NOW()` inline; SQLite binds an rfc3339 `now`.** Mirrors `claim_next`'s
  per-backend time handling so the lease-expiry comparison is identical.
- **REST first; MCP tool in 225.** Same split as 219 (REST) / 220 (MCP).

## Non-goals / deferred

- The MCP `get_queue_depth` tool (Cluster 225).
- Workspace-wide roll-up (a channel is the natural queue — `claim_next` is
  per-channel); can follow if an orchestrator needs the aggregate.

## Risks

- **Point-in-time only.** The counts are a snapshot; lease expiry advances with wall
  clock, so two calls a second apart can differ as leases lapse. That's inherent to
  the question and documented on the type.

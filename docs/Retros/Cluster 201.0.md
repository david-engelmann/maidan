# Cluster 201.0 retro — the bus stops shouting to every tenant

> Tag **`v201.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Arc D (performance & scale), part 4.

## What shipped

- `ShardedBroadcast`: the event bus routes a publish to the event's workspace
  shard + a global shard (cross-workspace subscribers), instead of one channel
  every subscriber filter-and-discards. Fan-out is now O(relevant subscribers),
  not O(all). Behavior unchanged; used by both buses.

## Surprises / decisions

- **The win is "don't wake them", not "don't deliver".** The old bus already
  delivered *correctly* — every subscriber filtered out other tenants' events.
  The cost was waking all of them to do it. Sharding doesn't change what any
  subscriber ends up seeing; it changes how many subscribers a publish touches.
  That framing is why this is safe: it's an efficiency layer *under* the
  authoritative `EventFilter`, not a change to delivery semantics. The unit test
  proves routing; the delivery e2es prove nothing regressed.
- **Global shard is not optional.** The tempting design is "one channel per
  workspace, done." But operators and any subscriber without a `workspace_id`
  filter need *all* events. So a publish fans out to the workspace shard **and**
  the global shard, and no-workspace subscribers live on global. Missing that
  would silently starve cross-workspace subscribers.
- **Prune on subscribe, not publish.** Shards must not accumulate forever (one
  per workspace ever seen). But pruning (a `retain` scan) on every publish would
  put an O(shards) scan on the hot path. Subscribe is far rarer, and it already
  takes the map lock — so prune there. The publish path only does an O(1)
  lookup.
- **The lazy-create race is real and handled.** A shard is created and its first
  receiver taken **under the same lock**, so `receiver_count()` is ≥ 1 before the
  lock releases — a concurrent prune (also under the lock) can't see 0 and drop
  a shard a subscriber just took. Getting this wrong would drop live subscribers.

## Decisions

- **Two Arc D items assessed and set down, not silently skipped.** Batched
  `pg_notify` is **declined**: the correct form needs range-hydration on the
  listener (it hydrates a single pointer today), a delivery-core change for a win
  that only helps the latency-tolerant fallback path — poor risk/value.
  Read-replica routing is **deferred**: it needs a read-pool threaded through a
  `Store` built around one pool, read-after-write handling, and a real replica to
  validate. Both are written up in Open Work with the design, so a future session
  starts from the analysis, not from scratch.

## Capability table extension

| Change | Where |
|--------|-------|
| Workspace-sharded event fan-out (`ShardedBroadcast`) | `maidan-bus/src/sharded.rs` + `inmem.rs` + `postgres.rs` |

## Risks identified + still open

- **Behavior-preserving, low risk** — the filter still runs; presence/resource
  notify are separate channels. Open: a no-`workspace_id` subscriber gets no
  sharding benefit (falls back to global — correct, unoptimized); shard-map
  growth is bounded by prune-on-subscribe but a very churny workspace set relies
  on subscribes happening to prune.

## Forward look

**Arc D's tractable perf wins are done** (198 harness, 199 concurrent context,
200 filtered-ANN, 201 sharded fan-out). The remaining listed items (batched
`pg_notify`, read-replica routing) are declined/deferred with rationale in Open
Work. Also open: federation `content→parts` egress (from Arc C 194), and the
perf follow-ups surfaced across 199–200.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Builds on the event bus
+ the [[Retros/Cluster 198.0]] harness.

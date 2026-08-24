# Cluster 265.0 retro — the rest of the reads, and a way to watch them

> Tag **`v265.0.0`**. Phase XXIV (post-gate hardening). **Program D — read-replica
> arc, part 5.** No new gate tag.

## What shipped

- 28 more content/collaboration read delegations routed to `read_pool()` (the whole
  member-facing read surface now offloads to the replica), plus
  `maidan_replica_reads_total{outcome=primary|replica}` so an operator can see the
  primary/replica split. Validated against real replication.

## Surprises / decisions

- **The carve-out is the real content of this cluster.** Routing "the rest of the
  reads" sounds mechanical, but the judgment is *which* reads must NOT route: the
  auth middleware runs on GETs (in-scope), so a session/token read on a lagging
  replica breaks auth immediately after minting — those stay on the primary.
  Same for config/ops reads (webhooks, slash, fsm-hooks, deliveries, reindex, audit,
  quotas): low offload value, high "why is my just-created thing missing?" surprise.
  Only content/collaboration reads route. Getting that boundary right is the work.
- **Router-internal reads are already safe.** `is_notification_muted` /
  `*_followers` are read by the background notification router, which is never in a
  request scope, so `read_pool` returns the primary for them regardless — left on
  `&self.pool` for clarity (and mute changes take effect immediately).
- **`\b` beat the substring traps.** Swapping `&self.pool` → `self.read_pool()` by
  `module::method(` prefix collided (`dm::` inside `group_dm::`, `members::` inside
  `channel_members::`); a `\b`-anchored regex re-pointed exactly the intended 28.
- **Metric stays out of the store.** Rather than a `metrics` dep in `maidan-store`,
  `PostgresStore` holds a plain two-atomic `ReadRoutingMetrics` that the server
  snapshots into Prometheus — the same shape as the bus's `HydrateStats`, keeping the
  store a pure data layer.

## Capability table extension

| Change | Where |
|--------|-------|
| 28 content-read delegations routed + `ReadRoutingMetrics` | `maidan-store/src/postgres/mod.rs` |
| `maidan_replica_reads_total` + AppState/main/metrics wiring | `state.rs`, `main.rs`, `metrics.rs` |

## Risks identified + still open

- Inert without a replica. Auth/control reads deliberately on the primary. Real-
  replica e2e asserts both routing outcomes; store tests unchanged.

## Forward look

**266** closes the arc: a replica-lag gauge + the Production.md "Read replicas"
section (config, the `Maidan-Consistency-Token` contract, ops guidance) + a final
validation sweep. Then the LSN read-replica arc — and Program D — is complete.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues [[Retros/Cluster 264.0]].

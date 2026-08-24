# Cluster 265.0 — route the remaining read families + routing metric (read-replica, part 5)

> **Program D (scale & durability) — read-replica arc, part 5.** Phase XXIV
> post-gate hardening. Tag **`v265.0.0`**. No new gate tag.

## Goal

Complete the read-routing surface: route the remaining content/collaboration reads
to the replica (Cluster 264 did the entity reads), and make routing observable with
`maidan_replica_reads_total{outcome}`.

## Scope

| Change | Where |
|--------|-------|
| Route 28 more content-read delegations to `read_pool()` (skills, results, notifications, follows, emails, last-seen, channel-members, dm/group-dm, transitions, queue-depth, task-schedules, assigned, deps, message-edits, mentions, inbox, votes, reactions, usage) | `postgres/mod.rs` |
| `ReadRoutingMetrics` (primary/replica counters) on `PostgresStore`, counted in `read_pool` | `postgres/mod.rs` |
| `maidan_replica_reads_total{outcome}` metric + `AppState.read_routing_metrics` + main.rs capture + sync | `state.rs`, `main.rs`, `metrics.rs` |

## Design decisions

- **Auth / control-plane reads stay on the primary — deliberately.** The auth
  middleware runs on GET requests too (which *are* in a read-consistency scope), so
  routing a session / API-token / OIDC / peer read to a lagging replica would fail
  auth right after a credential is minted. Those reads keep `&self.pool`. Config /
  ops reads (webhooks, slash-commands, fsm-hooks, automation-deliveries, reindex
  jobs, audit, token quotas) also stay on the primary — a just-created config read
  stale is confusing, and the offload value is low. Only **content/collaboration**
  reads route.
- **Router-internal reads left on the primary too.** `is_notification_muted`,
  `channel_followers`, `thread_followers` are read by the background notification
  router (never in a request scope, so `read_pool` returns the primary anyway) — left
  on `&self.pool` for clarity, and a just-set mute is always honored.
- **Substring-safe re-pointing.** A `\b`-anchored regex avoided the `dm::` vs
  `group_dm::` and `members::` vs `channel_members::` collisions when swapping
  `&self.pool` → `self.read_pool()` in the delegations.
- **Metric via the `HydrateStats` pattern.** `PostgresStore` holds an
  `Arc<ReadRoutingMetrics>` (two atomics), incremented in `read_pool` only when a
  replica is configured; the server captures it into `AppState` and the metrics loop
  emits `maidan_replica_reads_total{outcome=primary|replica}` as deltas — the store
  stays metrics-agnostic (no `metrics` dep).

## Validation

`route_decision` unit tests (CI) unchanged. The `#[ignore]`d `read_routing` e2e —
run against `scripts/replica-harness.sh` — **passes** and now also asserts the
routing counters saw both a primary read (token, replica behind) and a replica read
(no token, caught up).

## Non-goals / deferred

- A replica-lag gauge + Production.md "Read replicas" docs + the token contract
  (Cluster 266). Routing search (lives in maidan-search, a separate pool) — optional,
  deferred.

## Risks

- Inert without a replica; auth/control reads explicitly excluded. Store tests
  (called outside a scope → primary) are behaviour-identical; routing proven by the
  real-replica e2e.

# Cluster 264.0 — token ingestion + read routing (read-replica, part 4)

> **Program D (scale & durability) — read-replica arc, part 4.** Phase XXIV
> post-gate hardening. Tag **`v264.0.0`**. No new gate tag.

## Goal

Make replica reads real and safe: ingest the client's `Maidan-Consistency-Token`
and route a read to the replica only once it has replayed past that token — else the
primary. The heart of the arc.

## Scope

| Change | Where |
|--------|-------|
| `READ_CONSISTENCY` task-local + `with_read_consistency` scope helper | `postgres/mod.rs` |
| `PostgresStore` `read_pool()` selector + pure `route_decision` (+ unit tests) | `postgres/mod.rs` |
| Background replay-LSN poller (cached in an `AtomicU64`) | `postgres/mod.rs` |
| Re-point the entity-read delegations (workspace/member/channel/thread/message get+list) to `read_pool()` | `postgres/mod.rs` |
| Middleware: GET/HEAD → parse token → `with_read_consistency` scope | `consistency.rs` |
| Real-replica routing e2e (`#[ignore]`d) | `maidan-store/tests/read_routing.rs` |

## Design decisions

- **GET/HEAD-only routing sidesteps the read-then-write hazard.** Only pure read
  requests are wrapped in a read-consistency scope; mutation handlers are *not*, so
  any read-then-write inside a `POST/PUT/...` handler reads from the **primary** and
  never sees a stale replica. Outside any request (background workers) also reads the
  primary. So re-pointing a read method is always safe — it only routes when the
  request is a scoped GET.
- **Cached replay-LSN, not a per-read query.** A background poller refreshes the
  replica's `pg_last_wal_replay_lsn()` into an `AtomicU64` every 200 ms; `read_pool`
  compares the request's token against that cached value — a cheap atomic load, no
  extra round-trip per read. **A stale cache is safe:** a cached value is a *past*
  reading, so `cached ≤ actual`; `cached ≥ token ⟹ actual ≥ token`, i.e. the replica
  really has caught up. Staleness can only route to the primary unnecessarily
  (a false negative), never serve a stale read.
- **Token threading via a task-local, no signature churn.** `with_read_consistency`
  sets a `tokio::task_local`; read *delegations* in `mod.rs` call `self.read_pool()`
  instead of `&self.pool` (the free store fns are untouched — they still take a
  `&PgPool`). The routing decision is a pure `route_decision(has_replica, scope,
  cached)` function, unit-tested in CI.
- **Entity reads first.** This cluster routes the core get/list reads
  (workspace/member/channel/thread/message); the remaining read families (context,
  social, notifications, follows, skills, assignments, queue-depth, usage,
  transcripts) route in Cluster 265, plus metrics.

## Validation

Pure `route_decision` unit-tested in CI (5 cases: no-replica, out-of-scope,
no-token, caught-up, behind). The `#[ignore]`d `read_routing` e2e — run against
`scripts/replica-harness.sh` — **passes**: a token read returns the just-written row
(routed to the primary while the replica lags), the replica then replays past the
token, and a no-token read is served from the replica (row present on the standby).

## Non-goals / deferred

- Remaining read families + `maidan_replica_reads_total` metrics + a lag gauge
  (Cluster 265). Docs + ops (Cluster 266).

## Risks

- Inert without a replica (default). The GET-only scoping keeps every mutation
  handler's reads on the primary. Entity-read re-pointing guarded by the unchanged
  `http_crud_e2e` + `consistency_token_e2e`; routing proven by the real-replica e2e.

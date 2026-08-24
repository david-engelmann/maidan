# Cluster 264.0 retro — reads go to the replica, safely

> Tag **`v264.0.0`**. Phase XXIV (post-gate hardening). **Program D — read-replica
> arc, part 4.** No new gate tag.

## What shipped

- The routing heart: a `READ_CONSISTENCY` task-local (set by GET/HEAD handling via
  `with_read_consistency`), a `read_pool()` selector backed by a pure
  `route_decision`, a background poller caching the replica's replay LSN, and the
  entity-read delegations re-pointed to `read_pool()`. A read routes to the replica
  only once it has replayed past the client's token. Proven end-to-end against real
  streaming replication.

## Surprises / decisions

- **GET/HEAD-only scoping dissolved the scariest risk.** The nightmare in read
  routing is an internal read-then-write: a `POST` handler reads state, decides, then
  writes — if that read hit a lagging replica, the decision is on stale data. Rather
  than audit every handler, the middleware only opens a read-consistency scope for
  GET/HEAD. Mutation handlers are never in scope, so `read_pool` returns the primary
  for them unconditionally. That single rule makes re-pointing *any* read method safe
  and kills a whole class of subtle bugs.
- **A stale cache is a feature, not a bug.** The poller's cached replay LSN is always
  a *past* reading (`cached ≤ actual`), so `cached ≥ token` can only be true when the
  replica has *genuinely* caught up. Staleness produces false negatives (route to the
  primary when the replica was actually ready) — never a stale read. That's what lets
  the per-read decision be a single atomic load instead of a `pg_last_wal_replay_lsn()`
  round-trip on every read.
- **Re-point delegations, not module fns.** The store's free functions take a
  `&PgPool`; the `impl Store` block passes `&self.pool`. Routing is one edit per read
  *at the delegation* (`self.read_pool()`), leaving ~40 free fns untouched — and the
  writes obviously keep `&self.pool`. A pure `route_decision` carries the logic so CI
  covers it without a database.
- **Task-locals don't cross `tokio::spawn`.** The scope propagates to reads the
  handler `await`s inline (which is how these handlers call the store), but a read on
  a spawned sub-task would not see it. Fine here; worth remembering.
- **maidan-store now depends on `tokio`.** `task_local!` + the poller's `spawn`/`time`
  pulled tokio into the (previously runtime-free) store crate. It was already in the
  tree via sqlx; `cargo deny` stayed green.

## Capability table extension

| Change | Where |
|--------|-------|
| task-local + `with_read_consistency` + `read_pool`/`route_decision` + LSN poller + entity-read routing | `maidan-store/src/postgres/mod.rs` |
| GET/HEAD read-scope middleware | `maidan-server/src/consistency.rs` |

## Risks identified + still open

- Inert without a replica. GET-only scoping keeps mutation-handler reads on the
  primary. Real-replica e2e + `route_decision` unit tests + unchanged `http_crud_e2e`.

## Forward look

**265** routes the remaining read families + adds `maidan_replica_reads_total{outcome}`
+ a replica-lag gauge. **266** docs (Production.md "Read replicas" + the token
contract) + ops.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues [[Retros/Cluster 263.0]].

# Cluster 263.0 retro — handing the client its receipt

> Tag **`v263.0.0`**. Phase XXIV (post-gate hardening). **Program D — read-replica
> arc, part 3.** No new gate tag.

## What shipped

- `Store::write_lsn()` (Postgres `pg_current_wal_lsn()`, SQLite `None`), an
  `AppState.read_replica_enabled` flag, and a `consistency::middleware` that stamps
  `Maidan-Consistency-Token: <lsn>` on successful mutations when a replica is
  configured. The write half of the causality contract — Cluster 264 consumes it.

## Surprises / decisions

- **After-the-handler capture is not sloppy — it's correct.** The obvious worry is
  "the LSN I read after the handler isn't exactly this write's commit LSN." True, and
  it doesn't matter: another write can only push it *forward*, so the token is `>=`
  the real commit LSN, never `<`. Since a read is served from the replica only once
  it has replayed *past* the token, an over-approximated token can only cost a little
  extra wait or a primary fallback — never a stale read. The exact alternative
  (`RETURNING pg_current_wal_lsn()` inside every write tx) buys nothing here and
  would touch every mutating store method on both backends. One middleware chokepoint
  wins.
- **Gate the round-trip on a configured replica.** Without a replica the token is
  dead weight, so the middleware short-circuits on `read_replica_enabled` — the
  no-replica deployment pays nothing and behaves byte-identically. The flag mirrors
  the Cluster-183 `rate_limit_default_on` pattern (default false in `new`, set by
  `main.rs`).
- **`write_lsn` on the trait, not the pool, keeps the HTTP layer backend-agnostic.**
  SQLite returns `None` and the middleware just emits no header — no `if postgres`
  in the web layer.
- **Import-list gotcha.** `app.rs` pulls modules through a single `use crate::{...}`
  block; a new `consistency` module has to be added there (a bare `consistency::` ref
  otherwise fails to resolve), not just declared in `lib.rs`.

## Capability table extension

| Change | Where |
|--------|-------|
| `Store::write_lsn` | `store.rs`, `postgres/mod.rs`, `sqlite/mod.rs` |
| `read_replica_enabled` flag + `consistency::middleware` | `state.rs`, `main.rs`, `consistency.rs`, `app.rs` |

## Risks identified + still open

- Inert without a replica. Guarded by the Postgres-backed e2e (token present +
  parses on a mutation; absent on reads; absent when disabled) + a
  `--no-default-features` compile.

## Forward look

**264** ingests `Maidan-Consistency-Token` (middleware → task-local) and routes
replica-eligible reads via a `read_pool(token)` selector (replica when caught up,
else primary; replica error → primary). **265** validation + metrics. **266**
docs/ops.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues [[Retros/Cluster 262.0]].

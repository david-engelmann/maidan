# Cluster 262.0 — reader-pool split (read-replica, part 2)

> **Program D (scale & durability) — read-replica arc, part 2.** Phase XXIV
> post-gate hardening. Tag **`v262.0.0`**. No new gate tag.

## Goal

Give `PostgresStore` a distinct read pool, and connect a real read replica at boot
when one is configured — the plumbing the token-aware read router (Cluster 264)
will select against. Inert: reads still go to the primary.

## Scope

| Change | Where |
|--------|-------|
| `PostgresStore { pool, reader }` + `with_replica_reader` constructor + `reader()` accessor | `maidan-store/src/postgres/mod.rs` |
| `Config.replica_url` from `MAIDAN_DB_REPLICA_URL` | `maidan-server/src/config.rs` |
| Build the reader pool at boot (shared options) + `with_replica_reader` wiring | `maidan-server/src/main.rs` |
| `with_replica_reader` read/write smoke test | `maidan-store/tests/reader_pool.rs` |

## Design decisions

- **Default reader = the primary, so the ~62 `PostgresStore::new` call sites don't
  change.** `new(pool)` sets `reader: pool.clone()`; a real replica is supplied only
  via `with_replica_reader(pool, reader)`, which only `main.rs` calls when
  `MAIDAN_DB_REPLICA_URL` is set. Zero ripple, zero behaviour change when unset.
- **The reader is dormant until Cluster 264.** The field is `#[allow(dead_code)]`
  with a comment pointing at the router — the read methods still use `self.pool`.
  The token-aware selector (`read_pool(token)`) that chooses between `pool` and
  `reader` is 264's job; wiring it here would route reads to the replica
  *unconditionally* (no LSN check), which violates read-your-writes.
- **Shared pool options.** The primary and reader pools are built from one
  `make_pg_opts` closure, so the reader gets the same `statement_timeout`
  `after_connect` setup as the primary.
- **Fail-fast on a bad replica URL.** The reader pool connects at boot, so a
  misconfigured `MAIDAN_DB_REPLICA_URL` errors at startup (with context) rather than
  surfacing later — the one tangible behaviour this otherwise-inert cluster adds.

## Non-goals / deferred

- **Routing reads to the replica** (Cluster 264, once the LSN token flows).
- LSN capture + token header (Cluster 263).

## Risks

- Inert when `MAIDAN_DB_REPLICA_URL` is unset (the default) — reads unchanged.
  `backend_parity` (the Cluster-261 `replication` allowlist) + the `reader_pool`
  smoke + a `--no-default-features` compile guard the change.

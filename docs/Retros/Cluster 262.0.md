# Cluster 262.0 retro — a second pool, still asleep

> Tag **`v262.0.0`**. Phase XXIV (post-gate hardening). **Program D — read-replica
> arc, part 2.** No new gate tag.

## What shipped

- `PostgresStore { pool, reader }` + a `with_replica_reader` constructor;
  `MAIDAN_DB_REPLICA_URL` config; and boot-time wiring that connects a real reader
  pool when the env is set. Inert — reads still use the primary; the token-aware
  selector arrives in Cluster 264.

## Surprises / decisions

- **Default-preserving constructor beats a 62-site signature change.** `new(pool)`
  sets `reader = pool.clone()`, so every existing caller (mostly tests) is untouched
  and reads stay on the primary; only `main.rs` opts into a real replica via
  `with_replica_reader`. This is the same "add capability without rippling callers"
  move as the `AppState::attach_*` setters — the alternative (a required `reader`
  param) would have churned ~62 sites for no behavioural gain.
- **Deliberately dormant.** The temptation is to point reads at `self.reader` now
  ("it defaults to primary, so it's safe"). But once a replica *is* configured, a
  static `self.reader` would send every read to the replica with no LSN check —
  stale reads. So the field is `#[allow(dead_code)]` until 264 adds the per-call
  `read_pool(token)` selector. The one-cluster `#[allow]` is honest scaffolding, the
  same shape as Cluster-148's `request_client` (built, tested, no organic caller yet).
- **Fail-fast is the cluster's one real behaviour.** Connecting the reader pool at
  boot means a typo'd `MAIDAN_DB_REPLICA_URL` fails startup with context instead of
  producing mysterious routing errors later — worth having even before routing exists.
- **One options builder for both pools.** Factoring the `statement_timeout`
  `after_connect` into a `make_pg_opts` closure keeps the reader's connection setup
  identical to the primary's — no drift.

## Capability table extension

| Change | Where |
|--------|-------|
| `PostgresStore { pool, reader }` + `with_replica_reader` | `maidan-store/src/postgres/mod.rs` |
| `MAIDAN_DB_REPLICA_URL` + boot wiring | `maidan-server/src/config.rs`, `main.rs` |

## Risks identified + still open

- Inert by default (unset env → unchanged). Guarded by `reader_pool` smoke,
  `backend_parity`, and a `--no-default-features` compile.

## Forward look

**263** captures the primary's `pg_current_wal_lsn()` after a write and returns it as
a `Maidan-Consistency-Token` response header; **264** ingests that token and routes
replica-eligible reads via `read_pool(token)`; **265** validation + metrics; **266**
docs/ops.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues [[Retros/Cluster 261.0]].

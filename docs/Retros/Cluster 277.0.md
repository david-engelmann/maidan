# Cluster 277.0 retro — SQLite stops locking on the first write

> Tag **`v277.0.0`**. Phase XXIV (post-gate hardening). **Launch-readiness P0:
> SQLite write-contention deadlock.** No new gate tag.

## What shipped

The external review hit "database is locked" on the first workspace write with the
default SQLite config, and a one-connection pool was clean. This cluster reproduced it,
root-caused it, and fixed it.

- **Root cause (reproduced):** SQLite allows only one writer at a time, and sqlx's
  `pool.begin()` opens a *deferred* transaction. With more than one pooled connection,
  two writers each take a read snapshot (the `SELECT` scope our `*_with_event` methods
  do) and then race to upgrade to the writer — a genuine deadlock, which SQLite reports
  as `SQLITE_BUSY`/"database is locked" **immediately**, without honoring `busy_timeout`
  (waiting would never resolve it). A contention harness (`sqlite_write_contention`)
  measured a warm 8-connection pool failing **359 of 400** read-modify-write
  transactions; a single connection failed **0**.
- **Fix:** the SQLite backend now defaults to **one connection**
  (`maidan_store::DEFAULT_SQLITE_MAX_CONNECTIONS`, used by `main.rs`, overridable via
  `MAIDAN_DB_MAX_CONNECTIONS`). SQLite is the single-node / laptop / edge backend, so
  serializing through one connection is the correct and standard choice; WAL keeps it
  fast. Postgres (production / HA) is untouched and keeps its multi-connection pool.
- **Guard:** `default_sqlite_pool_survives_write_contention` runs the contention harness
  at the shipped default and asserts zero lock failures — so bumping the default back
  above 1 fails CI. An `#[ignore]`d `multi_connection_pool_deadlocks_under_contention`
  documents the bug on demand.

## Surprises / decisions

- **The pragmas were already right.** WAL + a per-connection 5 s `busy_timeout` + FK
  were all in place (Clusters 107/166), so this was not the obvious missing-pragma
  case. The trap is subtler: `busy_timeout` resolves *contention* (wait for a lock to
  free) but not a *deadlock* (two parties each waiting on the other), and deferred
  read-then-write transactions on multiple connections create exactly that deadlock.
- **Reproduce before fixing.** A minimal harness (raw sqlx, same pragmas, concurrent
  read-modify-write) turned a vague "it locked once" report into a measured 90% failure
  rate and a proven fix, and left a regression guard behind.
- **One connection, not `BEGIN IMMEDIATE`.** `BEGIN IMMEDIATE` would also fix it (take
  the write lock upfront so `busy_timeout` applies) and preserve read concurrency, but
  sqlx's `pool.begin()` is deferred with no per-transaction override, so that path
  means reworking every `*_with_event` begin site. For a single-node backend, one
  connection is the lower-risk, standard answer; the read-pool/`IMMEDIATE` refinement
  is logged in Open Work if SQLite read concurrency ever matters.

## Capability table extension

| Change | Where |
|--------|-------|
| SQLite pool defaults to 1 connection (`DEFAULT_SQLITE_MAX_CONNECTIONS`) | `maidan-store/src/lib.rs`, `maidan-server/src/main.rs` |
| contention reproduction + regression guard | `maidan-store/tests/sqlite_write_contention.rs` |

## Risks identified + still open

- **SQLite writes (and reads) now serialize through one connection.** Correct for the
  single-node backend, but a heavily concurrent SQLite deployment will queue on the
  connection (bounded by `acquire_timeout`). The docs already steer concurrent/HA use
  to Postgres. A read-pool + single-writer split is the follow-up if needed.

## Forward look

Two launch-readiness P0s are now closed (276 version, 277 SQLite first-write). Remaining
"Public-launch readiness" items: the one-command quickstart, `maidan init`,
LangChain/AutoGen recipes + interop CI, a published benchmark, and A2A v1.0 compliance.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues
[[Retros/Cluster 276.0]]. Finding from the external launch-readiness review (Cluster 274).

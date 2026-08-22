# Cluster 261.0 retro — proving we can test it before we build it

> Tag **`v261.0.0`**. Phase XXIV (post-gate hardening). **Program D (scale &
> durability), part 4 — read-replica arc, part 1.** No new gate tag.

## What shipped

- The `Lsn` causality-token type (`maidan-types`), the store LSN helpers
  (`postgres::replication`), and `scripts/replica-harness.sh` + an `#[ignore]`d
  test that validates the helpers against **real** streaming replication. The
  keystone that unblocks the whole LSN read-replica arc — the ability to test it.

## Surprises / decisions

- **The deferral was never about the code — it was about validation.** Read-replica
  routing was put off three times because "it needs a real replica to validate." So
  the first cluster builds the replica: a proven local pgvector streaming-replication
  harness. Everything after this can be tested against an actual standby, which is
  the whole reason the arc is now tractable.
- **The two-node replication recipe took a spike to nail.** Two non-obvious gotchas,
  both now baked into `replica-harness.sh`: (1) `POSTGRES_HOST_AUTH_METHOD=trust`
  does **not** authorize *replication* connections — pg_hba needs an explicit
  `host replication all all trust` line + reload; (2) the standby's `postgres` must
  run as the `postgres` user (it refuses to run as root), so the custom
  `pg_basebackup -R && exec postgres` entrypoint runs `--user postgres`.
- **`Lsn` is a `u64`, and that's the point.** The obvious representation is the
  `pg_lsn` text `X/Y`, but it mis-orders as a string (`0/9` > `0/10` lexically), and
  the token exists *only* to answer a `>=` comparison. Parsing to `u64` up front makes
  the comparison correct and trivial; the `0/9 < 0/10` unit test guards it.
- **Infra in the script, assertions in the test.** Rather than fight the
  testcontainers API to orchestrate two-node replication + `pg_basebackup`, the
  script owns the infra and prints two URLs, and the `#[ignore]`d test (skipping when
  the URLs are unset) owns the assertions — the loadgen external-URL pattern. Robust,
  and reusable by every later routing cluster.

## Capability table extension

| Change | Where |
|--------|-------|
| `Lsn` token type + tests | `maidan-types/src/lsn.rs` |
| `current_wal_lsn` / `replica_replay_lsn` / `replica_caught_up` | `maidan-store/src/postgres/replication.rs` |
| `replica-harness.sh` + validated `#[ignore]`d test | `scripts/replica-harness.sh`, `maidan-store/tests/replication.rs` |

## Risks identified + still open

- None active — the helpers are inert (not wired into any read path yet).

## Forward look

**262** reader-pool split (inert — `PostgresStore { pool, reader }`, default
reader=primary, `MAIDAN_DB_REPLICA_URL`), then **263** LSN capture + token header,
**264** token ingestion + read routing, **265** comprehensive validation + metrics,
**266** docs/ops.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues [[Retros/Cluster 260.0]].

# Cluster 261.0 — LSN primitives + replication harness (read-replica, part 1)

> **Program D (scale & durability), part 4 — read-replica arc, part 1.** Phase XXIV
> post-gate hardening. Tag **`v261.0.0`**. No new gate tag.

## Goal

Validate-first. Before any read-routing code, build (a) the causality-token type
the whole arc turns on, and (b) a way to stand up **real** streaming replication
locally so every later cluster can be tested against an actual replica — the exact
thing whose absence caused read-replica routing to be deferred three times.

## Scope

| Change | Where |
|--------|-------|
| `Lsn` causality-token type (`u64`-backed, `pg_lsn` parse/display, numeric `Ord`) + unit tests | `maidan-types/src/lsn.rs` |
| Store LSN helpers `current_wal_lsn` / `replica_replay_lsn` / `replica_caught_up` | `maidan-store/src/postgres/replication.rs` |
| `scripts/replica-harness.sh` — local pgvector primary + streaming standby | `scripts/replica-harness.sh` |
| `#[ignore]`d test validating the helpers against a real standby | `maidan-store/tests/replication.rs` |

## Design decisions

- **The token is a `u64`, not the `pg_lsn` text.** `pg_lsn`'s `X/Y` text does **not**
  order as a string (`0/9` vs `0/10`); the numeric form does. Since the token's whole
  job is the comparison "has the replica replayed past my write?", `Lsn(u64)` with
  `from_pg_str` / `to_pg_str` is the correct representation. This is unit-tested (the
  `0/9 < 0/10` case is the canonical trap).
- **LSN helpers are called directly, not via the `Store` trait.** Like the bus's
  `events::get_by_id`, these are a Postgres-streaming-replication concern with no
  SQLite analogue, so they live in `postgres::replication` and are `pub`-reachable —
  no trait method, no SQLite twin, no ripple to the 62 `PostgresStore::new` sites.
  `replica_replay_lsn` returns `Option` (NULL on a primary, which is not in recovery);
  `replica_caught_up` is the router's one-call predicate.
- **Infra in a script, assertions in the test (the loadgen pattern).**
  `replica-harness.sh up` stands up the proven pgvector replication pair and prints
  `MAIDAN_PRIMARY_URL` / `MAIDAN_REPLICA_URL`; the `#[ignore]`d test connects to
  those (skips when unset, so a normal `cargo test` is unaffected). This decouples
  the fiddly two-node replication setup from the Rust assertions and gives every
  later read-routing cluster the same validation vehicle.
- **The replication recipe (proven locally).** pgvector/pgvector:pg17; primary with
  `wal_level=replica` + a `host replication all all trust` pg_hba line (host-auth
  trust does *not* cover replication); standby via `pg_basebackup -R` run as the
  `postgres` user (postgres refuses root). Validated: standby `replay_lsn` reaches
  the primary's `current_wal_lsn`, replicated rows visible.

## Validation

`eval "$(scripts/replica-harness.sh up)"` then
`cargo test -p maidan-store --test replication -- --ignored` → **passes** against a
real standby (helpers correct; standby catches up to the token; rows replicate). The
pure `Lsn` tests run in CI.

## Non-goals / deferred (later in the arc)

- 262 reader-pool split (inert). 263 LSN capture on writes + token header. 264 token
  ingestion + read routing. 265 comprehensive validation + metrics. 266 docs/ops.

## Risks

- Delivery-adjacent but inert here — the helpers aren't wired into any read path yet,
  so zero behaviour change; only new types + a manual harness.

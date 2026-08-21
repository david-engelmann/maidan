# Cluster 259.0 — chaos / fault-injection harness

> **Program D (scale & durability), part 2.** Phase XXIV post-gate hardening.
> Tag **`v259.0.0`**. No new gate tag.

## Goal

Prove the Cluster-258 self-healing NOTIFY floor actually heals a *real* fault, not
just the synthetic gap its unit test covers. A chaos harness drives publishes at a
`PostgresBus` while repeatedly killing the `LISTEN` backend connection, and asserts
every published event still reached the local broadcast — the floor back-filling
whatever the dropped notifications would have delivered.

## Scope

| Change | Where |
|--------|-------|
| `fault_due(op, every)` pure fault-schedule + CI unit tests | `crates/maidan-bus/tests/chaos.rs` |
| `terminate_listener_backends(pool)` — `pg_terminate_backend` on the `LISTEN` connection | `crates/maidan-bus/tests/chaos.rs` |
| `#[ignore]`d soak `notify_floor_survives_periodic_listener_kills_under_load` | `crates/maidan-bus/tests/chaos.rs` |
| `scripts/chaos.sh` env-knobbed runner | `scripts/chaos.sh` |

## Design decisions

- **Same measure-first shape as the Cluster-198 load harness.** The end-to-end
  scenario is `#[ignore]`d — it needs Docker and is timing-sensitive, so it is a
  resilience *tool* run explicitly (`scripts/chaos.sh` / `--ignored`), never a
  pass/fail CI gate that could flake a required job. The pure `fault_due` schedule
  helper *is* unit-tested in CI.
- **Kill the listener, not the pool.** `terminate_listener_backends` targets only
  backends whose `query` is a `LISTEN` (and never `pg_backend_pid()`), so it drops
  the bus's listener connection while leaving the append/publish pool intact — the
  precise fault the floor is built to survive.
- **Assert on the set of delivered `log_id`s, tolerating duplicates.** A background
  subscriber collects every delivered id into a set; after the run plus a settle
  window (covering the ~1 s reconnect drain), every *published* id must be present.
  Duplicates are expected and fine (the floor + at-least-once both tolerate
  re-delivery); a *missing* id is the failure.
- **Env-knobbed like loadgen.** `MAIDAN_CHAOS_OPS` / `MAIDAN_CHAOS_KILL_EVERY` /
  `MAIDAN_CHAOS_DELAY_MS` tune the soak.

## Validation

Run locally against a Postgres testcontainer: **40 published, 40 delivered, 5
listener kills, 0 missing** — the floor healed every one of five backend
terminations with no lost events.

## Non-goals / deferred

- DB-pause / whole-container kill faults (need container-runtime control beyond
  `pg_terminate_backend`); the listener-kill is the on-mission fault for the floor.
- Other Program D items: backup/restore + DR runbook, read-replica routing.

## Risks

- The soak is `#[ignore]`d, so it never gates CI; only the pure `fault_due` tests
  run there. The harness is a manual/operator tool, consistent with `loadgen`.

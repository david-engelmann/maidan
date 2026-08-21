# Cluster 259.0 retro — kill the listener, keep the events

> Tag **`v259.0.0`**. Phase XXIV (post-gate hardening). **Program D (scale &
> durability), part 2.** No new gate tag.

## What shipped

- A chaos / fault-injection harness (`crates/maidan-bus/tests/chaos.rs` +
  `scripts/chaos.sh`): an `#[ignore]`d soak that publishes under load while
  repeatedly terminating the `LISTEN` backend, and asserts every published event
  still reached the local broadcast — proving the Cluster-258 floor heals a real
  disconnect. The pure `fault_due` schedule helper is unit-tested in CI.

## Surprises / decisions

- **The floor works — measured, not asserted.** The 258 unit test proves the
  *mechanism* (a `backfill` call drains a synthetic gap); this cluster proves the
  *system*: with `pg_terminate_backend` killing the listener five times mid-soak, all
  40 published events still arrived (0 missing). That end-to-end confidence is the
  whole point of a chaos harness — the mechanism test can't tell you the reconnect
  path actually fires and re-`LISTEN`s.
- **`#[ignore]`d on purpose, like loadgen.** A timing-sensitive, Docker-dependent
  soak has no business gating a required CI job (the coverage-flake and
  never-a-flaky-gate lessons). So the scenario is a manual tool; only the pure
  `fault_due` cadence helper runs in CI. This keeps the harness honest — it can be
  imperfect/slow without ever reddening `main`.
- **Kill precisely.** Terminating *all* backends would take the append/publish pool
  down with the listener and prove nothing. Filtering `pg_stat_activity` to
  `query ILIKE 'LISTEN%'` (excluding `pg_backend_pid()`) drops exactly the listener
  connection — the fault the floor targets.
- **Assert on a set, tolerate dups.** The floor may re-deliver an event (a back-fill
  racing a live NOTIFY); the correctness property is "no event is *missing*," so the
  assertion is set-containment of published ⊆ delivered, not equality.
- **`clippy::manual_is_multiple_of`.** `op % every == 0` is now a clippy lint under
  `-D warnings` (newer toolchain); used `op.is_multiple_of(every)` (stable since
  1.87, we pin 1.91).

## Capability table extension

| Change | Where |
|--------|-------|
| Chaos harness: `fault_due` + `terminate_listener_backends` + `#[ignore]`d soak | `crates/maidan-bus/tests/chaos.rs` |
| `scripts/chaos.sh` runner | `scripts/chaos.sh` |

## Risks identified + still open

- Container-level faults (DB pause, whole-container kill) are out of reach for a
  `pg_terminate_backend`-based harness; the listener-kill is the on-mission fault.

## Forward look

Program D remaining: backup/restore + DR runbook, and the larger read-replica
routing refactor (single-pool `Store` → reader/writer; needs a real replica to
validate).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues [[Retros/Cluster 258.0]].

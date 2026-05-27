# Cluster 10.0 retro — Postgres transactional outbox

> Closing wave for Cluster 10.0 · target tag `v10.0.0`.

Cluster 10.0 narrowed the crash window between event-log commit and bus
publish on Postgres by enqueueing `maidan_outbox` in the same transaction
as `maidan_events` and relaying pending rows to `PostgresBus` after commit.

## What shipped

- **PR #169** — Cluster kickoff plan ([[Clusters/Cluster 10.0]]).
- **PR #170** — Implementation bundle (10.0.1–10.0.4):
  - Migration `0013_outbox.sql`; transactional `append_event` + outbox enqueue.
  - `OutboxRelay` background task; `publish()` defers `bus.publish` on Postgres.
  - Federation ingest fixed (no double `append_event`).
  - Metrics `maidan_outbox_pending`, `maidan_outbox_relay_total{result}`.
  - Integration tests; Decisions/Architecture/Production docs.

## What was deferred

| To          | What                                              | Why                                      |
|-------------|---------------------------------------------------|------------------------------------------|
| Post-10.0   | NOTIFY / LISTEN guaranteed delivery               | Outbox covers commit→publish ordering only. |
| Post-10.0   | SQLite outbox                                     | Postgres-only scope.                     |
| Post-10.0   | Consumer dedup table                                | Subscribers use `log_id` + replay today.   |
| Post-10.0   | Coverage floor 11%+                                 | Separate cluster.                        |

## Surprises

- Federation `ingest_envelope` called `append_event` then `publish`, which
  appended twice; routing through `publish()` only fixed it.
- Relay duplicate publishes are acceptable: pointer NOTIFY is idempotent by `log_id`.

## Decisions

- **Postgres-only outbox** — SQLite/InMemory keep in-process append-then-publish.
- **Relay at-least-once** — mark published only after successful `bus.publish`;
  failed rows stay pending with incremented `attempts`.

## Capability table extension

| Capability                                              | First available in |
|---------------------------------------------------------|--------------------|
| Transactional outbox on Postgres (`maidan_outbox`)        | `v10.0.0`          |
| Outbox relay background task                            | `v10.0.0`          |
| Outbox metrics on `/metrics`                            | `v10.0.0`          |

## Risks identified + mitigated

- **Crash between append and NOTIFY** — row + outbox commit together; relay publishes after.
- **Federation double-append** — single `publish()` path.

## Risks identified + still open

- **NOTIFY fire-and-forget** — unchanged; relay can retry and duplicate notifies.
- **Stuck pending rows** — ops runbook for high `maidan_outbox_pending`.
- **Coverage depth** — floor still 10.5%.

## Forward look

Next: coverage 11%+ measured bump, or consumer-side dedup / stronger delivery
semantics. See [[Open Work]].

## Acknowledgements

Solo cluster. Kickoff #169, implementation #170, this retro.

# Cluster 14.0 retro — SQLite transactional outbox

> Closing wave for Cluster 14.0 · target tag `v14.0.0`.

Cluster 14.0 brought the transactional outbox pattern to SQLite so `append_event` and
`maidan_outbox` enqueue commit together; a relay publishes to `InMemoryBus` after commit,
with the same quarantine semantics as Postgres.

## What shipped

- **PR #179** — Implementation bundle (14.0.1–14.0.4):
  - Migration `migrations/sqlite/0013_outbox.sql`.
  - `sqlite/outbox.rs`, transactional `sqlite/events::append`.
  - `OutboxBackend` enum; `OutboxRelay` and metrics work on both dialects.
  - SQLite enabled for outbox relay in `main.rs`.
  - Integration tests (`outbox_sqlite`, `outbox_sqlite_http_e2e`).
  - Kickoff plan [[Clusters/Cluster 14.0]]; Decisions/Architecture/Production updates.

## What was deferred

| To          | What                                              | Why                                      |
|-------------|---------------------------------------------------|------------------------------------------|
| Cluster 15  | Epic pick (semantic SQLite, MCP subscribe, S3)    | One epic per cluster.                    |
| Post-14.0   | SQLite delivery cursors                           | Postgres-first in 13.0.                  |

## Surprises

- `OutboxBackend` wrapper avoided duplicating the full relay loop.

## Decisions

- **SQLite relay → InMemoryBus** — ordering fix for single-process dev; not multi-node fan-out.
- **Combined outbox + quarantine schema** — SQLite migration 13 includes `quarantined_at` from day one.

## Capability table extension

| Capability                                              | First available in |
|---------------------------------------------------------|--------------------|
| SQLite transactional outbox + relay                     | `v14.0.0`          |
| `OutboxBackend` (Postgres \| SQLite) for relay/metrics    | `v14.0.0`          |

## Risks identified + mitigated

- **Crash between SQLite append and bus publish** — same TX + relay as Postgres.

## Risks identified + still open

- **Multi-process SQLite** — still one process + in-memory bus; production scale uses Postgres.
- **NOTIFY semantics** — unchanged on Postgres.

## Forward look

Next: **Cluster 15.0** — epic pick at retro (SQLite semantic, MCP `resources/subscribe`, S3 multipart).

## Acknowledgements

Solo cluster. Implementation #179, this retro.

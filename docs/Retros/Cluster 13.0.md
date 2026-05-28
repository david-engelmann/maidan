# Cluster 13.0 retro — Delivery contract & subscriber ledger

> Closing wave for Cluster 13.0 · target tag `v13.0.0`.

Cluster 13.0 added durable per-consumer delivery cursors on Postgres and wired them into
federation ingest and optional WebSocket / MCP SSE subscribe paths, with documented
at-least-once semantics.

## What shipped

- **PR #178** — Implementation bundle (13.0.1–13.0.4):
  - Migration `0015_delivery_cursor.sql`; `get_delivery_cursor` / `advance_delivery_cursor`.
  - Federation advances `federation:{peer_id}` after successful ingest.
  - Optional `consumer_id` on WS subscribe and MCP SSE; replay floor from cursor.
  - Decisions, Architecture, Production delivery contract sections.
  - Postgres integration tests; stabilized bus-lag WS e2e drain.

## What was deferred

| To          | What                                              | Why                                      |
|-------------|---------------------------------------------------|------------------------------------------|
| Cluster 14  | SQLite transactional outbox                       | Separate epic.                           |
| Post-13.0   | SQLite delivery cursors                           | Postgres-first scope.                    |
| Post-13.0   | Dedicated federation/WS postgres e2e for cursors  | Store tests cover cursor semantics.      |

## Surprises

- WS lag-replay e2e was flaky when the outbound queue filled; background drain fixed it.

## Decisions

- **Monotonic cursor only** — `GREATEST` on advance; no rewind via API.
- **Postgres-first** — SQLite stubs return 0 / no-op until a later cluster needs them.

## Capability table extension

| Capability                                              | First available in |
|---------------------------------------------------------|--------------------|
| `maidan_delivery_cursor` per consumer + workspace       | `v13.0.0`          |
| Optional `consumer_id` on WS / MCP subscribe            | `v13.0.0`          |
| Federation delivery cursor (`federation:{peer_id}`)     | `v13.0.0`          |

## Risks identified + mitigated

- **Cursor hides events if regressed** — monotonic advance only; docs state replay floor behavior.

## Risks identified + still open

- **NOTIFY / outbox at-least-once** — unchanged; cursors reduce duplicate replay for registered consumers only.
- **Exactly-once expectations** — explicitly out of scope in docs.

## Forward look

Next: **Cluster 14.0** — SQLite transactional outbox. See [[Clusters/Cluster 14.0]].

## Acknowledgements

Solo cluster. Implementation #178, this retro.

# Cluster 83.0 retro — SQLite delivery cursor

> Tag **`v83.0.0`**.

## What shipped

- `maidan_delivery_cursor` on SQLite (migration `0023`, since **`v56.0.0`**) with monotonic `get` / `advance` in `sqlite/delivery_cursor.rs`.
- Store trait wired on `SqliteStore`; federation and event delivery use the same cursor path as Postgres.
- `delivery_cursor` integration tests exercise Postgres (testcontainers) and in-memory SQLite watermarks.

## What was deferred

- HTTP operator API to list/reset cursors (SQL in [[Production]] remains the path).
- Cross-workspace cursor admin UI.

## Next

Cluster **84** — outbox relay modes ([[Clusters/Product Ladder 77+]]).

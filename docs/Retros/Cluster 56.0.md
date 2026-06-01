# Cluster 56.0 retro — Delivery guarantees

> Tag **`v56.0.0`**.

## What shipped

- SQLite migration `0023_delivery_cursor` and store parity with Postgres cursors.
- `OutboxBackend::replay_quarantined` (Postgres + SQLite).
- `POST /workspaces/:wid/outbox/:oid/replay` with audit `outbox.replay`.

## What was deferred

- MCP tool for outbox replay (HTTP is enough for operators).
- Listing quarantined rows over HTTP (metrics + SQL remain).

## Forward look

Cluster **57**: Agent app model (installed apps with scoped capabilities).

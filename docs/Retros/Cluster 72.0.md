# Cluster 72.0 retro — A2A task streaming

> Tag **`v72.0.0`**.

## What shipped

- Postgres/SQLite tables for workspace push URLs and serialized A2A tasks.
- `SubscribeToTask` / `tasks/resubscribe` SSE on `POST /a2a/v1/rpc`.
- Push config set/get uses store (replaces in-memory `a2a_push` map).
- Best-effort outbound POST on task persistence.

## What was deferred

- Long-running tasks with multi-frame subscribe until terminal (Maidan still completes synchronously).
- Signed push payloads (integrators should use network controls).
- `SubscribeToTask` on tasks that transition WORKING→COMPLETED mid-stream.

## Forward look

Cluster **71** (event contract) and **73** (MCP streamable) complete Phase XII transport depth.

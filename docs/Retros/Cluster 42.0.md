# Cluster 42.0 retro — Presence & typing

> Tag **`v42.0.0`**.

## What shipped

- `PresenceHub` (in-memory per workspace) on `AppState`.
- WebSocket subscribe frame optional `member_id` (requires `filter.workspace_id`).
- Outbound: `presence_snapshot`, `presence` (online/away/offline), `typing`.
- Inbound client frames for presence status and typing start/stop.

## What was deferred

- HTTP presence API and persistence across server restarts.
- MCP stream typing/presence (WS-only for v42).
- Per-channel presence (workspace-scoped fan-out).

## Forward look

Cluster **43**: UI v2 shell.

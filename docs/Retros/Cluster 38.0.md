# Cluster 38.0 retro — MCP resource fan-out complete

> Tag **`v38.0.0`**.

## What shipped

- `publish_resource_uris` on HTTP `PATCH /messages/:id`, `POST /workspaces/:id/purge`,
  `POST /messages/:id/mentions`, and `POST /messages/:id/votes`.
- `resource_updates::uris_for_message` and `uris_for_workspace_purge` helpers.
- E2E: edit message triggers `notifications/resources/updated` on subscribed thread URI.

## What was deferred

- Per-channel/thread fan-out listing on workspace purge (workspace URI only).
- WebSocket resource notification transport.

## Forward look

Phase II Cluster **39**: direct messages schema + HTTP/MCP.

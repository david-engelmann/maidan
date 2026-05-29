# Cluster 49.0 retro — Agent context export

> Tag **`v49.0.0`**.

## What shipped

- `GET /threads/:id/context` returns messages, references, artifacts, FSM state.
- `Store::list_thread_transitions` for transition history.
- Artifact SHAs discovered from message `metadata` (`artifact_sha256`, `artifacts[]`).

## What was deferred

- MCP `get_thread_context` tool (HTTP sufficient for v49).
- UI context panel.

## Forward look

Cluster **50**: outbound webhooks.

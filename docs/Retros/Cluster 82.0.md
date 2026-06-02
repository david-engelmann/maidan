# Cluster 82.0 retro — Context pagination

> Tag **`v82.0.0`**.

## What shipped

- `Store::list_messages_after` (sqlite + postgres) with `posted_at ASC, id ASC` ordering.
- HTTP thread/workspace context: `message_cursor`, `thread_cursor`, `next_message_cursor`, `next_thread_cursor`.
- MCP `get_thread_context` / `get_workspace_context` pagination args; results use standard MCP `content[]` envelope.
- `context_pagination_e2e`; ordering table in [[Agent Integration]].

## What was deferred

- Per-thread `message_cursor` inside workspace context packs (workspace pagination is thread-level only).
- Store-level thread listing cursor (workspace context still loads threads then slices in memory).

## Next

Cluster **83** — SQLite delivery cursor ([[Clusters/Product Ladder 77+]]).

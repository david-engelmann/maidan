# Cluster 39.0 retro — Direct messages

> Tag **`v39.0.0`**.

## What shipped

- `maidan_dm_conversations` table (ordered member pair + dedicated thread) on Postgres and SQLite.
- Private `__dm__` channel per workspace for DM threads.
- HTTP: open/list conversations, post/list messages.
- MCP tools for open, list, and post.
- `EventFilter.dm_conversation_id` expands to `thread_id` for `/ws/subscribe` and `/mcp/stream`.
- `MessagePosted` / edit / tombstone events carry optional `dm_conversation_id`.

## What was deferred

- Group DMs and multi-party conversations.
- Dedicated `maidan://dm/{id}` MCP resource read path.
- Inbox/unread (Cluster 40).

## Forward look

Cluster **40**: mention router, `GET /members/:id/inbox`, delivery preferences.

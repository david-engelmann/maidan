# Cluster 40.0 retro — Mention router & inbox

> Tag **`v40.0.0`**.

## What shipped

- `maidan_inbox_cursor` per member (`last_read_at` watermark) on Postgres and SQLite.
- `GET /members/:id/inbox` and `POST /members/:id/inbox/read` with enriched mention items and `unread_count`.
- `maidan-router` baseline `@handle` parsing and auto-`record_mention` on HTTP/MCP message post.
- Store `list_member_inbox` / `advance_inbox_last_read_at`.

## What was deferred

- Per-member delivery preferences (mute channels, mention-only).
- DM rows as first-class inbox items without an `@` mention.
- MCP `get_member_inbox` tool.

## Forward look

Cluster **41**: reactions and pins.

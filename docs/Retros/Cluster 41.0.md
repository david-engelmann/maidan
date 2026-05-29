# Cluster 41.0 retro — Reactions & pins

> Tag **`v41.0.0`**.

## What shipped

- `maidan_reactions` (per message/member/emoji) and `maidan_pins` (per thread/message).
- HTTP: `POST/GET/DELETE /messages/:id/reactions`, `POST/GET/DELETE /threads/:id/pins`.
- Events: `ReactionAdded`, `ReactionRemoved`, `MessagePinned`, `MessageUnpinned`.
- MCP tools for reactions and pins; resource fan-out on mutations.

## What was deferred

- Reaction aggregates / summary counts in list-messages responses.
- Channel-level pins (thread-scoped only).
- Custom emoji registry.

## Forward look

Cluster **42**: presence and typing indicators on WebSocket.

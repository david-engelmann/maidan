# Cluster 20.0 retro — Message router

> Closing wave for Cluster 20.0 · target tag `v20.0.0`.

Cluster 20.0 centralized channel/thread/message hierarchy resolution in
`maidan-router` for HTTP and MCP.

## What shipped

- **PR #192** — `resolve_channel_context`, `resolve_thread_context`,
  `resolve_message_chain`; server routes and MCP resource fan-out wired through
  the crate; SQLite integration test.

## What was deferred

| To         | What                         | Why                          |
|------------|------------------------------|------------------------------|
| Cluster 21 | Google A2A protocol surface  | Separate transport epic.     |
| Cluster 27 | MCP streamable HTTP          | Transport finalization.      |

## Forward look

Next: **Cluster 21.0** — A2A agent transport. Ladder:
[[Clusters/Product Ladder 17-27]].

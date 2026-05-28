# Cluster 20.0 — Message router

Cluster 19.0 closed S3 multipart artifacts at **`v19.0.0`**. The server today
duplicates `get_thread` → `get_channel` chains for auth scoping and event context.

> **Goal:** Centralize channel / thread / message hierarchy resolution in
> `maidan-router`; HTTP and MCP call into it.
>
> **Target tag:** `v20.0.0`.

## PRs

| #          | Title                                              | Issue |
|------------|----------------------------------------------------|-------|
| kickoff    | `docs: Cluster 20.0 kickoff` (this doc)              | —     |
| 20.0.1     | `feat(maidan-router): resolve channel/thread/message` | TBD   |
| 20.0.2     | `feat(maidan-server): use maidan-router in routes` | TBD   |
| 20.0.3     | `feat(maidan-mcp): router-backed resource fan-out` | TBD   |
| 20.0.retro | `docs(retro): Cluster 20.0 + v20.0.0 tag prep`       | TBD   |

## Exit criteria

- `resolve_channel_context`, `resolve_thread_context`, `resolve_message_chain` tested on SQLite.
- Server message/thread handlers use `maidan-router` (no local duplicate chains).
- `v20.0.0` tagged after retro.

## Out of scope

- Mention parsing from message bodies.
- A2A transport (Cluster 21).

## References

- [[Clusters/Product Ladder 17-27]], [[Retros/Cluster 19.0]].

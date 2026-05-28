# Cluster 17.0 — MCP resource fan-out

Cluster 16.0 closed HTTP notification transport at **`v16.0.0`**. Subscribers still
only receive updates for `post_message` → `maidan://threads/{id}`.

> **Goal:** Notify subscribed resource URIs for all MCP tool mutations that change
> thread, channel, workspace, or artifact resources.
>
> **Target tag:** `v17.0.0`.

## PRs

| #          | Title                                                                  | Issue |
|------------|------------------------------------------------------------------------|-------|
| kickoff    | `docs: Cluster 17.0 kickoff` (this doc + ladder)                       | —     |
| 17.0.1     | `feat(maidan-mcp): resource URI resolution for tool mutations`         | TBD   |
| 17.0.2     | `test: MCP subscribe fan-out integration`                              | TBD   |
| 17.0.3     | `docs: fan-out in Architecture + MCP reference`                          | TBD   |
| 17.0.retro | `docs(retro): Cluster 17.0 + v17.0.0 tag prep`                         | TBD   |

## Exit criteria

- CI green on `main`.
- Subscribers to thread, channel, workspace, and artifact URIs receive notifications
  for the mapped MCP tools.
- `v17.0.0` tagged after retro.

## Out of scope

- SQLite semantic search (Cluster 18).
- HTTP routes outside MCP tools (use separate epic if needed later).

## References

- [[Clusters/Cluster 16.0]], [[Clusters/Product Ladder 17-26]].

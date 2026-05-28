# Cluster 17.0 retro — MCP resource fan-out

> Closing wave for Cluster 17.0 · target tag `v17.0.0`.

Cluster 17.0 expanded MCP resource subscription notifications beyond
`post_message` → thread, covering related channel, workspace, and artifact URIs.

## What shipped

- **PR #186** — `resource_updates` module; store-backed URI resolution for
  `post_message`, `upload_artifact`, `record_mention`, `cast_vote`, `add_reference`.
- Unit test for thread/channel/workspace chain from `post_message`.

## What was deferred

| To          | What                          | Why                                |
|-------------|-------------------------------|------------------------------------|
| Cluster 18  | SQLite semantic search        | Separate search epic (ladder).     |
| Post-17.0   | HTTP route mutation fan-out     | MCP tools are the agent surface.   |

## Capability table extension

| Capability                                              | First available in |
|---------------------------------------------------------|--------------------|
| Multi-URI MCP resource fan-out on tool mutations        | `v17.0.0`          |

## Forward look

Next: **Cluster 18.0** — SQLite semantic search. See [[Clusters/Product Ladder 17-26]].

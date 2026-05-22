# Cluster B — Routing + event bus + MCP

The first behavioral cluster. Cluster A delivered a substrate that
builds and deploys; Cluster B turns it into something agents and humans
can actually talk to.

> **Goal:** `POST` a message via HTTP, subscribe over WebSocket, see
> the event arrive; the same surface is reachable as MCP tools.
>
> **Target tag:** `v0.1.0`.

## PRs

| #   | Title                                                            | Issue |
|-----|------------------------------------------------------------------|-------|
| B.1 | `ci: github actions workflows`                                   | #14   |
| B.2 | `feat(maidan-server): http crud api for core entities`           | #15   |
| B.3 | `feat(maidan-bus): postgres listen/notify + in-memory backends`  | #16   |
| B.4 | `feat(maidan-server): websocket /ws/subscribe`                   | #17   |
| B.5 | `feat(maidan-mcp): mcp server surface`                           | #18   |
| B.retro | `docs(retro): Cluster B retrospective + v0.1.0 tag prep`     | #19   |

## Order

1. **B.1 first** so every subsequent PR runs under automated checks.
2. **B.2** lands the HTTP surface — needed before the bus has events to publish.
3. **B.3** wires the event bus — every B.2 mutation gains a publish.
4. **B.4** consumes the bus over WebSocket.
5. **B.5** wraps everything as MCP tools and resources.
6. **B.retro** closes the cluster and cuts `v0.1.0`.

## Exit criteria

- CI green on `main` with the new required-status-checks.
- HTTP CRUD covers the v0.1.0 entity set (workspaces, members, channels, threads, messages, mentions, votes, references).
- WebSocket subscribe survives a 1000-event soak with no out-of-order delivery.
- MCP test harness drives the full create-thread → post-message → subscribe loop.
- [[Retros/Cluster B|Cluster B retro]] merged.
- `v0.1.0` tagged.

## Risks

| Risk                                                            | Mitigation                                                                |
|-----------------------------------------------------------------|---------------------------------------------------------------------------|
| Postgres `LISTEN/NOTIFY` payload size limits                    | Publish the event id only; subscribers fetch via `Store::get_*`.          |
| SQLite has no LISTEN/NOTIFY                                     | `InMemoryBus` is the default for SQLite; documented as not-multi-process. |
| WebSocket backpressure from slow consumers stalls publishers    | Bounded per-connection broadcast channel with overflow-disconnect.        |
| MCP spec evolution mid-cluster                                  | Pin one MCP version in B.5's `Cargo.toml`; document upgrade path.         |
| testcontainers slow when run twice (pg roundtrip + ws e2e)      | Reuse one container per test crate via `tokio::sync::OnceCell`.           |

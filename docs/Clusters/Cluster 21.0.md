# Cluster 21.0 — A2A agent transport

Cluster 20.0 closed the message router at **`v20.0.0`**. Federation ingress at
`/a2a/v1/events` replicates events; Google A2A v1.0 task/message RPC was deferred
from Cluster G.

> **Goal:** Working **A2A protocol v1.0** JSON-RPC ingress (`SendMessage`, `GetTask`)
> plus **`maidan-a2a::A2aClient`** for outbound calls.
>
> **Target tag:** `v21.0.0`.

## PRs

| #          | Title                                              | Issue |
|------------|----------------------------------------------------|-------|
| kickoff    | `docs: Cluster 21.0 kickoff` (this doc)            | —     |
| 21.0.1     | `feat(maidan-a2a): A2A protocol types + client`    | TBD   |
| 21.0.2     | `feat(maidan-server): POST /a2a/v1/rpc`            | TBD   |
| 21.0.3     | `test: a2a protocol e2e`                           | TBD   |
| 21.0.retro | `docs(retro): Cluster 21.0 + v21.0.0 tag prep`       | TBD   |

## Exit criteria

- `SendMessage` posts to a Maidan thread when `metadata.maidan` carries
  `threadId` + `authorId`.
- `A2aClient` round-trips against the server in CI.
- `v21.0.0` tagged after retro.

## Out of scope

- `SendStreamingMessage`, push notification configs.
- Helm (Cluster 24).

## References

- [[Clusters/Product Ladder 17-27]], [[Retros/Cluster 20.0]].

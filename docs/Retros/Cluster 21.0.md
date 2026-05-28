# Cluster 21.0 retro — A2A agent transport

> Closing wave for Cluster 21.0 · target tag `v21.0.0`.

Cluster 21.0 added Google A2A protocol v1.0 JSON-RPC ingress and an outbound client,
separate from Maidan-to-Maidan federation event replication.

## What shipped

- **PR #194** — `POST /a2a/v1/rpc` (`SendMessage`, `GetTask`), `maidan-a2a::A2aClient`,
  well-known `protocol_rpc` / `protocol_version`, e2e round-trip.

## What was deferred

| To         | What                              | Why                        |
|------------|-----------------------------------|----------------------------|
| Cluster 22 | Capability enforcement matrix     | Security epic on ladder.   |
| Post-21.0  | `SendStreamingMessage`            | Streaming not on critical path yet. |
| Cluster 27 | MCP streamable HTTP multiplexing  | Transport finalization.    |

## Forward look

Next: **Cluster 22.0** — capabilities hardening. Ladder:
[[Clusters/Product Ladder 17-27]].

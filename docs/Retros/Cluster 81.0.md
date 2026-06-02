# Cluster 81.0 retro — Subscribe grants v3

> Tag **`v81.0.0`**.

## What shipped

- `channel_grants` on WS subscribe and `GET /mcp/stream` query params.
- `subscribe_grants` enforces private-channel deny + allow-list; DM subscribe auto-grants the DM thread channel.
- `EventFilter` blocks private `channel_created` and channel-scoped events without grants.
- `contracts/ws-subscribe-filter.schema.json` v3; `subscribe_grants_e2e`.

## What was deferred

- Hot-updating grants without resubscribe.
- Per-member grant ACLs beyond channel private flag.

## Next

Cluster **82** — context pagination ([[Clusters/Product Ladder 77+]]).

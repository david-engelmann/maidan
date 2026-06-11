# Presence and workspace roster (Cluster 99)

## Workspace roster HTTP

- `GET /workspaces/:wid/members` — list members (requires `workspace:read`).
- Browser session path: `GET /ui/api/workspaces/:wid/members` (OIDC session or bearer).

Use the roster to populate operator UI pickers and to validate `member_id` on WS subscribe frames.

## WebSocket presence fan-out

After subscribing on `GET /ws/subscribe`, include both:

- `filter.workspace_id` — required for replay and workspace-scoped events.
- `member_id` — your workspace member UUID (enables presence/typing).

Optional control frames from the client:

```json
{"type":"presence","status":"online"}
{"type":"typing","thread_id":"<uuid>","active":true}
```

Server frames: `presence_snapshot`, `presence`, `typing` (see OpenAPI `/ws/subscribe` description).

Authentication: bearer token with `event:subscribe`, **or** a valid `maidan_session` cookie on the WS handshake (Cluster 93).

## Multi-replica behavior (Cluster 103)

On Postgres (NOTIFY mode), presence/typing and the roster are consistent across
server replicas: a member online on one replica appears in another's
`presence_snapshot` and live frames. Internally each replica publishes changes
on the `maidan_presence` channel and keeps a merged, TTL-expiring view of
members on other replicas, refreshed by a heartbeat.

- A disconnect propagates an `offline` frame promptly; a crashed replica's
  members expire from others' rosters within the TTL.
- Tunables: `MAIDAN_PRESENCE_HEARTBEAT_SECS` (default 10), `MAIDAN_PRESENCE_TTL_SECS`
  (default 30) — see [[Production]].
- Delivery is at-most-once (as with the event bus); a dropped delta is
  reconciled by the next heartbeat. In single-process / SQLite deployments
  presence stays local (no behavioral change).

## Related

- [[Agent Integration]] — HTTP capability map
- [[Clusters/Cluster 99.0]] — cluster exit criteria

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

## Related

- [[Agent Integration]] — HTTP capability map
- [[Clusters/Cluster 99.0]] — cluster exit criteria

# Cluster 154.0 — `request_client` GET-stream delivery fix

**Theme:** Lane 3 (of the user's three-lane plan), part 1. Make server→client
requests reach the spec-canonical `GET /mcp/streamable` stream — they previously
reached only a client that held a POST SSE leg.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v154.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| Per-session `client_req_tx` broadcast + `push_client_request` + `subscribe_client_requests` | `crates/maidan-mcp/src/streamable_session.rs` |
| `request_client` delivers via `push_client_request` (not the POST-leg mpsc) | `crates/maidan-mcp/src/server.rs` |
| `stream_get` subscribes the GET leg to the session request broadcast + merges it | `crates/maidan-server/src/mcp_streamable.rs` |

## Why

`request_client` (Cluster 148) pushed the server→client JSON-RPC request onto
the session's **POST-leg mpsc** — the single-consumer channel drained by the
POST response SSE. The MCP spec's server→client channel is the standing **GET**
stream, and `stream_get` consumed only the *global* `subscribe_notifications`
broadcast — it never saw `request_client` pushes. So a client that connected the
canonical GET stream and waited for a sampling / elicitation request would wait
forever.

Because the session channel is a single-consumer mpsc, the GET stream couldn't
simply share it. A per-session **broadcast** dedicated to server→client requests
lets any number of GET legs receive them without disturbing the POST-leg
response/notification path.

## Non-goals

- **`Last-Event-ID` replay of server→client requests** on the GET stream —
  requests are delivered live to a connected GET leg; no leg → `request_client`
  fails fast. Notification replay (Cluster 147) is unchanged.
- **A real caller** — deferred to Cluster 155 (needs session context in tool
  dispatch); `request_client` still has no organic caller after this cluster.

## PR ladder (actual)

| # | Title |
|---|--------|
| 154.0.1 | `fix(mcp): deliver server→client requests on the canonical GET stream` (#398) |
| 154.0.retro | `docs(retro): Cluster 154.0 + v154.0.0 tag prep` |

## Exit criteria

- A GET-stream subscriber receives a `request_client` push; no subscriber →
  fail fast; the round-trip e2e reads the request off the GET stream — **met**.
- `v154.0.0` tagged after retro.

## Verification & limits

- Unit: `client_requests_reach_a_get_stream_subscriber`. E2E:
  `server_to_client_request_round_trips_over_http` now opens a GET stream and
  reads the request there. `request_client` unit tests read via
  `subscribe_client_requests`.
- Limit: behavior change — server→client requests no longer reach a POST-only
  client (spec-canonical GET-only delivery). No organic caller, so no regression.

## References

- [[Retros/Cluster 154.0]]; [[Clusters/Cluster 153.0]]; `streamable_session.rs`,
  `mcp_streamable.rs`, `server.rs` (`request_client`). The streamable subset's
  substantive record is the **Cluster 35.0 retro**.

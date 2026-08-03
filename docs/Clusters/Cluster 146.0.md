# Cluster 146.0 — GET /mcp/streamable server→client SSE + Accept negotiation

**Theme:** Second slice of the **MCP streamable spec-completeness arc (145–148)**
— the streamable endpoint's server-initiated GET stream and `Accept`-based
content negotiation.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v146.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| `GET /mcp/streamable` — server→client SSE stream (unsolicited notifications), session-aware | `mcp_streamable.rs::stream_get`, `app.rs`, cap-map, OpenAPI desc |
| `Accept` negotiation on `POST /mcp/streamable` — JSON-only clients get a single JSON body, not an SSE session | `mcp_streamable.rs::accepts_event_stream` |

## Design

- The GET stream reuses the existing `McpServer::subscribe_notifications()`
  broadcast — the same source the POST-opened SSE fans in — so it's a thin,
  session-aware wrapper (touch/echo an open `Mcp-Session-Id`), not new plumbing.
- `Accept` negotiation is a top-of-handler branch on `POST /mcp/streamable`: no
  `text/event-stream` (and no `*/*`) → answer with a single JSON response.

## Non-goals (deferred within the arc)

- **Resumability** (`Last-Event-ID` replay) — 147. The GET stream delivers
  live notifications but doesn't yet replay missed ones.
- **Per-session notification scoping** — the GET stream carries the server-wide
  notification broadcast (matching the POST-opened SSE); a per-session filter
  is a possible later refinement, not a spec requirement here.

## PR ladder (actual)

| # | Title |
|---|--------|
| 146.0.1 | `feat(mcp): GET /mcp/streamable server→client SSE + Accept negotiation` (#382) |
| 146.0.retro | `docs(retro): Cluster 146.0 + v146.0.0 tag prep` |

## Exit criteria

- GET stream delivers a server notification; JSON-`Accept` POST returns JSON;
  cap-map/OpenAPI/matrix contracts pass — **met**.
- `v146.0.0` tagged after retro.

## Verification & limits

- E2E: `streamable_get_delivers_server_notification`,
  `streamable_post_with_json_accept_returns_single_json`. `fmt`/`clippy` clean;
  contract tests (cap-map/OpenAPI/matrix) pass with the new `surface: mcp` GET
  entry.

## References

- [[Retros/Cluster 146.0]]; [[Clusters/Cluster 145.0]]; `mcp_streamable.rs`,
  `mcp_notifications.rs` (the broadcast→SSE pattern reused).

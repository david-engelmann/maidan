# Cluster 146.0 retro — GET /mcp/streamable server→client SSE + Accept negotiation

> Tag **`v146.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Second of the MCP streamable spec-completeness arc (**145–148**).

## What shipped

- **`GET /mcp/streamable`** — the spec's server→client SSE stream on the
  streamable endpoint, delivering unsolicited server notifications from the
  server-wide `subscribe_notifications()` broadcast; touches + echoes an open
  `Mcp-Session-Id` (`workspace:read`).
- **`Accept`-header content negotiation on `POST /mcp/streamable`** — a
  JSON-only client (`Accept` without `text/event-stream`) gets a single JSON
  response instead of an opened SSE session; absent `Accept` keeps the
  streaming default.
- Wired the GET route + a `surface: mcp` cap-map entry; refreshed the OpenAPI
  description (also covering the 145 batching/version work).

## What was deferred / not covered

| To | What | Why |
|----|------|-----|
| 147 | Resumability (`Last-Event-ID` replay) | The GET stream is live-only; replay of missed events needs a per-session event-id log. |
| Future | Per-session notification scoping | The GET stream carries the server-wide broadcast (matching the POST-opened SSE); a per-session filter isn't a spec requirement here. |

## Surprises

- **The GET stream needed no new plumbing.** It subscribes to the same
  notification broadcast the POST-opened SSE already fans in, so it's a thin,
  session-aware wrapper over an existing source.
- **Three `.route("/mcp/streamable", …)` calls merge cleanly** (POST/GET/DELETE)
  — axum merges same-path `MethodRouter`s when the methods don't overlap.

## Decisions

- **Reuse the broadcast, don't invent a session-scoped queue** — the spec's GET
  stream is for unsolicited server messages, which are exactly the notifications
  already broadcast.
- **Accept negotiation as a top-of-handler branch** — JSON-only → single body;
  everything else unchanged, so the streaming default is preserved.

## Capability table extension

| Capability | Where |
|------------|-------|
| `GET /mcp/streamable` server→client SSE; `Accept`-based JSON vs SSE on POST | `mcp_streamable.rs`, `contracts/http-capability-map.json` |

## Risks identified + still open

- **Low.** Additive route + an opt-in negotiation branch; the SSE-default POST
  behavior is unchanged (the pre-existing streamable e2e passes untouched).

## Forward look

**147** next: resumability — SSE `id:` fields + `Last-Event-ID` replay of missed
messages after a reconnect (needs a per-session event log). Then **148** the
server→client request layer.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].

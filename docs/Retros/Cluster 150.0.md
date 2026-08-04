# Cluster 150.0 retro — MCP stream thread/member/kind filters

> Tag **`v150.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Second of the MCP-agent-surface arc (149 inbox/mentions + 150 stream filters).

## What shipped

- Four query params on `GET /mcp/stream` — `channel_id`, `thread_id`,
  `member_id`, and `kinds` (comma-separated snake_case, parsed via
  `EventKind::parse`; unknown → `400`) — wired into the `EventFilter` in
  `resolve_stream_params`. Delivers the "await my mention" primitive
  (`?workspace_id=…&member_id=…&kinds=mention_recorded`).

## What was deferred / not covered

| To | What | Why |
|----|------|-----|
| n/a | New filter fields | All already exist on `EventFilter`; this only exposes them on the MCP query surface. |
| Future | Repeated-key `kinds` array | axum `Query` is `serde_urlencoded` (no repeated→Vec); comma-separated is the simple, unambiguous encoding. |

## Surprises

- **Exactly parallel to 149.** Both clusters closed the same shape of gap: a
  fully-built backend capability (the inbox reads; the `EventFilter` fields)
  that the WebSocket/HTTP surface had but the MCP surface silently didn't
  expose. Two small omissions with an outsized effect on agent autonomy.

## Decisions

- **Comma-separated `kinds`** over a JSON body or repeated keys — matches the
  query-string transport and parses cleanly with the existing `Query`
  extractor.
- **Reject unknown kinds (400)** rather than silently drop — a mistyped kind
  should fail loudly, not yield an empty stream.

## Capability table extension

| Capability | Where |
|------------|-------|
| `GET /mcp/stream` narrowing by channel/thread/member/kind | `mcp_stream.rs` |

## Risks identified + still open

- **Low.** Additive optional query params over the existing filter/matches
  machinery; absent = today's behavior.

## Forward look

The MCP-agent-surface pair (149 + 150) is complete: an MCP-only agent can now
**discover** its @mentions (inbox tools) and **await** them in real time
(stream filters). Queued next-arc: **B1** lean read modes (token efficiency),
**C1** a live-updating `/ui` thread view, and **D** the `request_client` fix +
a real caller.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].

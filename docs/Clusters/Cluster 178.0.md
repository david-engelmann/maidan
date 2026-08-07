# Cluster 178.0 — token: opt-in lean event frames

**Theme:** Arc 4 (token round 3), part 4 (final) — a subscribe flag that trims
the streamed domain-event frames. **Completes token round 3 and the post-v155
four-arc program.**

**Ladder:** Post-gate — **Phase XXIV**, tag **`v178.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| `lean` subscribe flag → domain-event frames carry `{log_id, kind, ...ids}` instead of the full event | `ws.rs`, `mcp_stream.rs`, `event_stream.rs` |

## Why

WS / MCP-SSE subscribers received the **full** serialized event on every frame
(for a `message_posted`, the whole `Message` with body + content). An agent that
just tails for activity — "did anything happen in my thread?" — and reads on
demand paid for the entire payload on every event. `lean=true` swaps each frame
for a `LeanFrame` pointer: `{log_id, kind, workspace_id?, channel_id?,
thread_id?, member_id?}`.

## How

A `lean` flag on the subscribe request (WS `SubscribeFrame` / MCP-SSE query) is
threaded through `forward_bus_items`, `replay_matching_events`, and
`reconcile_deliver` (the same plumbing `at_least_once` uses). At the three
frame-serialize sites, `frame_payload(envelope, lean)` emits either the full
flattened envelope or a `LeanFrame` built from the `Event` accessors
(`kind()`/`workspace_id()`/`channel_id()`/`thread_id()`/`member_id()`).

## Key decisions

- **Strict-subset frame → drop-in compatible.** A full frame is
  `{log_id, kind, <event fields…>}` (flattened, internally-tagged). The lean
  frame keeps exactly the top-level routing fields clients already read
  (`log_id`, `kind`, `thread_id` — e.g. the Cluster 153 `/ui` live-refresh) and
  drops only the heavy embedded payload. So lean is transparent to that logic.
- **Opt-in, default off.** No change for existing subscribers.
- **Works on every delivery path** — optimistic live, lag-replay, and the
  at-least-once reconcile loop all route through `frame_payload`.

## Non-goals

- Per-kind field selection / GraphQL-style projections — the lean frame is a
  fixed pointer shape.

## Exit criteria

- A `lean` subscriber's event frames omit the embedded payload but keep the
  routing ids; suites green — **met**.
- `v178.0.0` tagged. **Token round 3 + the four-arc program are complete.**

## Verification & limits

- `mcp_stream_lean_frames_omit_the_event_payload` (mcp_stream_at_least_once_e2e):
  a `lean=true` subscriber's first frame keeps `log_id`/`kind`/`workspace_id` but
  has no embedded `workspace` payload. All frame-consuming suites
  (ws_subscribe / mcp_stream / mcp_streamable / ui_ws_tail) stay green (default
  path unchanged).
- Limit: a lean tail is a pointer — the client must fetch (REST / a non-lean
  read) to get bodies. That's the intended trade.

## References

- [[Retros/Cluster 178.0]]; `event_stream.rs` (`LeanFrame` / `frame_payload`),
  `ws.rs`, `mcp_stream.rs`. Program: [[Roadmap]] + memory
  `maidan-next-arc-program`.

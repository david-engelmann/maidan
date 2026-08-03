# Cluster 147.0 — MCP streamable resumability (SSE event ids + Last-Event-ID)

**Theme:** Third slice of the **MCP streamable spec-completeness arc (145–148)**
— per-stream event ids and `Last-Event-ID` reconnect/redelivery.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v147.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| Session SSE frames carry a monotonic `id:`; bounded per-session replay log | `maidan-mcp/src/streamable_session.rs` |
| `GET /mcp/streamable` with `Last-Event-ID` replays retained frames after that id, then live | `mcp_streamable.rs::stream_get` |
| Session survives a dropped POST stream (reconnect possible); follow-up degrades to inline JSON if the leg is gone | `mcp_streamable.rs` |

## Design

- `SessionEntry` gains `next_event_id` + a bounded `VecDeque` log; the channel
  carries `(event_id, payload)`; `replay_after(id, after)` returns the retained
  frames with a higher id.
- The GET stream is `chain(replay, live)`: replay the `Last-Event-ID` set, then
  the live broadcast — no separate store.
- Removing close-on-drop lets the session (and its log) outlive its POST leg;
  the follow-up path answers inline (200) when the leg is gone rather than
  500'ing (the response is logged for replay regardless).

## Non-goals / limits

- **Live follow-up-POST responses after a reconnect** still target the original
  leg — a session has one live channel. Replay covers the missed-events case
  (the primary resumability value); a fully re-attachable live stream is a
  later refactor.
- The GET stream's live broadcast frames don't carry session event ids (only
  replayed + POST-leg frames do); the spec allows id-less events.

## PR ladder (actual)

| # | Title |
|---|--------|
| 147.0.1 | `feat(mcp): streamable resumability — SSE event ids + Last-Event-ID replay` (#384) |
| 147.0.retro | `docs(retro): Cluster 147.0 + v147.0.0 tag prep` |

## Exit criteria

- Frames carry `id:`; GET replays after `Last-Event-ID`; session survives drop;
  tests green — **met**.
- `v147.0.0` tagged after retro.

## Verification & limits

- Unit: `replay_after` (only-newer, empty tails, unknown session). E2E:
  `streamable_get_replays_after_last_event_id`. `fmt`/`clippy` clean; the mux /
  GET / Accept / capability e2e are unaffected.

## References

- [[Retros/Cluster 147.0]]; [[Clusters/Cluster 146.0]];
  `maidan-mcp/src/streamable_session.rs`, `mcp_streamable.rs`.

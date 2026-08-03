# Cluster 147.0 retro — MCP streamable resumability

> Tag **`v147.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> Third of the MCP streamable spec-completeness arc (**145–148**).

## What shipped

- **SSE event ids + bounded replay log** — the session registry assigns a
  monotonic per-session id to each pushed frame and retains the last 256 in a
  `VecDeque` for replay; the SSE channel now carries `(event_id, payload)` and
  the POST-opened stream renders each as `id:`.
- **`Last-Event-ID` replay on `GET /mcp/streamable`** — reconnecting with the
  header replays the session's retained frames after that id (id'd SSE events),
  then continues live (`chain(replay, live)`). New `replay_after`.
- **The session survives a dropped POST stream** — removed close-on-drop so a
  client can reconnect; TTL/DELETE still clean up. A follow-up POST whose leg is
  gone answers inline (200) instead of 500, and the response is logged for
  replay either way.

## What was deferred / not covered

| To | What | Why |
|----|------|-----|
| Future | Re-attachable live stream after reconnect | A session has one live channel (the POST leg); live follow-up responses after a reconnect still target it. Replay covers the missed-events case. |
| n/a | Session event ids on the GET live-broadcast frames | Only replayed + POST-leg frames are id'd; the spec permits id-less events. |

## Surprises

- **Resumability forced a lifecycle change.** For reconnect to be possible the
  session (and its log) has to outlive its POST leg — so close-on-drop had to
  go, which in turn made the follow-up path degrade to inline JSON when the leg
  is gone (it previously 500'd). Net-more-correct, but not an obvious knock-on.
- **Redelivery is a two-line stream combinator.** Because the log lives in the
  session entry, `Last-Event-ID` replay is just `chain(replay, live)` — no new
  store, no cursor table.

## Decisions

- **Log before deliver** in `push` — the frame is recorded for replay even when
  the live `try_send` fails, so a client that already dropped can still recover
  it on reconnect.
- **Bounded log (256)** — same bound as the live buffer; replay is best-effort
  over a recent window, not an unbounded journal.

## Capability table extension

| Capability | Where |
|------------|-------|
| MCP streamable SSE `id:` + `Last-Event-ID` reconnect replay | `maidan-mcp/src/streamable_session.rs`, `mcp_streamable.rs` |

## Risks identified + still open

- **Session longevity after drop** — dropped sessions now linger until TTL
  (1h default) rather than closing immediately; `prune_expired` (lazy) + DELETE
  bound the memory, and the per-session log is capped at 256.

## Forward look

**148** concludes the arc: server→client requests (sampling / roots /
elicitation) + per-session client-capability tracking (captured from
`initialize`). That's the last spec gap.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].

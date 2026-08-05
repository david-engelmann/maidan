# Cluster 153.0 retro — live-updating `/ui` thread view

> Tag **`v153.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> UI polish — next-arc lane 2 (of the user's three-lane plan).

## What shipped

- **The open thread updates live.** In `wsSocket.onmessage`, a domain-event
  frame (`typeof log_id === "number"`) whose `thread_id === selectedThreadId`
  and whose `kind` is a thread-content kind (`message_posted` /
  `message_edited` / `message_tombstoned` / `reaction_added` /
  `reaction_removed` / `message_pinned` / `message_unpinned`) calls
  `scheduleLiveRefresh()` → a debounced `loadMessages()` (≤1 reload / 300 ms).
- **`● live` indicator** flashes green on each refresh; hidden/reset on thread
  switch.
- **Static guard** `ui_js_wires_live_thread_refresh` in `tests/ui_js_contract.rs`.

## What was deferred / not covered

| To | What | Why |
|----|------|-----|
| Possible follow-up | Auto-scroll message pane to the newest message on refresh | Kept re-render identical to manual "Reload"; scroll behavior is a separate polish. |
| By design | A dedicated per-thread WS | Reused the existing subscribe socket; live updates require its filter to include the thread. |

## Surprises

- **The frames were already usable.** `Event` is `#[serde(tag = "kind")]`
  (internally-tagged), so each WS frame is flat — `{log_id, kind, thread_id,
  channel_id, message, …}`. No backend change was needed to know which frames
  touch the open thread; the data had been flowing to the Events tab all along,
  just not routed into the message pane.

## Decisions

- **Fixed-window coalescing** (`if (liveRefreshTimer) return;` + a 300 ms timer)
  over trailing debounce — a continuous burst still refreshes at a steady ≤1/300 ms
  instead of being starved by a trailing reset.
- **Gate on `thread_id` + a curated kind set**, not "any frame for the thread" —
  avoids needless reloads on non-content events (e.g. `thread_state_changed`)
  while covering everything that changes the visible message list.

## Capability table extension

| Capability | Where |
|------------|-------|
| Live `/ui` thread view from WS frames | `crates/maidan-server/static/index.html` |

## Risks identified + still open

- **Low.** UI-only; the WS handler's other branches are untouched (the log line
  still appends). Depends on the subscribe filter including the thread —
  surfaced in the tooltip. No browser in CI, so behavior rests on the static
  guard + manual check (the standing `/ui` limitation).

## Forward look

Lanes 1 (token efficiency, 151+152) and 2 (live thread view, 153) of the user's
three-lane plan are done. Lane 3 — `request_client` — splits into two clusters
because the tool-dispatch path carries no session id and the session channel is
a single-consumer mpsc only the POST leg drains: **154** fixes GET-stream
delivery (per-session broadcast so the spec-canonical `GET /mcp/streamable`
receives server→client requests), then **155** adds a real caller
(session-context threading + a sampling-backed `summarize_thread`).

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].

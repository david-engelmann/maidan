# Cluster 153.0 — live-updating `/ui` thread view

**Theme:** UI polish (next-arc lane 2). The console already received every
workspace WS event but only rendered domain-event frames as log lines; make the
open thread's message pane update live from them.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v153.0.0`**, no new gate tag.

---

## Scope

| Change | Where |
|--------|-------|
| WS domain-event frames for the open thread → debounced `loadMessages()` | `static/index.html` (`wsSocket.onmessage`, `scheduleLiveRefresh`, `liveFrameTargetsOpenThread`) |
| `● live` indicator that flashes on refresh + resets on thread switch | `static/index.html` (markup + CSS + `selectThread`) |
| Static guard for the wiring | `tests/ui_js_contract.rs` (`ui_js_wires_live_thread_refresh`) |

## Why

The subscribe stream delivered all events, but a domain-event frame only landed
as a `[log_id] kind` line in the Events tab — the Threads message pane never
changed until you hit "Reload messages". Since the frames are internally-tagged
(`kind` + a flat `thread_id`), the client can tell which frames touch the open
thread and refresh in place.

## Non-goals

- **A dedicated per-thread WebSocket** — the feature rides the existing subscribe
  socket; live updates require its filter to include the thread (workspace-wide
  covers it). Documented in the `● live` tooltip.
- **Backend changes** — none; this is pure `/ui` JS/CSS.
- **Auto-scroll-to-latest** on refresh — possible polish follow-up.

## PR ladder (actual)

| # | Title |
|---|--------|
| 153.0.1 | `feat(ui): live-updating thread view — WS frames refresh messages` (#396) |
| 153.0.retro | `docs(retro): Cluster 153.0 + v153.0.0 tag prep` |

## Exit criteria

- A message/reaction/pin event for the open thread refreshes the pane
  (debounced); other threads' frames are ignored; contract tests green — **met**.
- `v153.0.0` tagged after retro.

## Verification & limits

- `tests/ui_js_contract.rs`: the existing undefined-helper guard plus a new
  `ui_js_wires_live_thread_refresh` that asserts the helper is defined, invoked,
  gated by `liveFrameTargetsOpenThread`, and driven by the kind set. No browser
  in CI — this is the established `/ui` test model (Cluster 133/143).
- Limit: static-only verification of behavior; live delivery is exercised by
  hand against a running server.

## References

- [[Retros/Cluster 153.0]]; `static/index.html`, `crates/maidan-types/src/events.rs`
  (`Event` is `#[serde(tag = "kind")]`), `crates/maidan-server/src/ws.rs`.

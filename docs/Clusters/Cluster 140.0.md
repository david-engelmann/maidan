# Cluster 140.0 — Workspace presence roster in the console

**Theme:** Surface the realtime presence hub in the `/ui` — a "Presence" tab
showing who's online, fed by the `presence_snapshot` frames that already ride
the existing WebSocket subscribe.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v140.0.0`**, no new gate tag.

---

## Scope

| Layer | Deliverable |
|-------|-------------|
| **Backend** | None. Presence has no HTTP API — it's purely WS (`presence_snapshot` server frames + `{"type":"presence"}` client frame over `/ws/subscribe`). |
| **UI (index.html)** | A `panel-presence` view: a roster rendered from each `presence_snapshot` frame (member id · status, "you" marked) and Online/Away buttons that send the presence frame over the open socket. |

## How it works (no new wiring)

The WS subscribe already sends `member_id` when the operator is signed in,
which registers them in the presence hub. The frames already arrived — they
were just dumped raw into the events log. This cluster renders the
`presence_snapshot` into a roster and adds a status control. Nothing new
under `/ui/api`.

## Non-goals

- Typing indicators in the UI — the `typing` frame is logged but not
  surfaced; a per-thread typing affordance can follow.
- A separate presence WS connection — the roster reflects the single socket
  owned by the Events tab; opening a second socket would double-register.

## PR ladder (actual)

| # | Title |
|---|--------|
| 140.0.1 | `feat(ui): workspace presence roster in the console` (#370) |
| 140.0.retro | `docs(retro): Cluster 140.0 + v140.0.0 tag prep` |

## Exit criteria

- Presence roster renders from `presence_snapshot`; Online/Away send over the
  socket; guard green — **met**.
- `v140.0.0` tagged after retro.

## Verification & limits

- `ui_js_contract` guard validates the new JS; no Rust change. Per the UI
  track's standing limit, JS *behavior* is inspection-verified (no browser);
  the `presence_ws` e2e covers the protocol.

## References

- [[Retros/Cluster 140.0]]; `static/index.html` (`renderPresence`/`setPresence`,
  the `presence_snapshot` branch in `connectWs`'s `onmessage`), `presence.rs`,
  `ws.rs`.

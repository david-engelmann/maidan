# Cluster 140.0 retro — Workspace presence roster in the console

> Tag **`v140.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.

## What shipped

- **"Presence" tab in `index.html`** (`panel-presence`): a live roster
  rendered from each `presence_snapshot` frame (member id · status, with
  "you" marked), plus Online/Away buttons that send
  `{"type":"presence","status":...}` over the open WebSocket.
- **`onmessage` render hook**: the existing `presence_snapshot` branch now
  calls `renderPresence(v)` (in addition to logging), so the roster updates
  on every snapshot.

## What was deferred / not covered

| To | What | Why |
|----|------|-----|
| Future | Typing indicators in the UI | The `typing` frame is logged but not surfaced; a per-thread affordance can follow. |
| n/a | A presence-specific WS connection | The roster reflects the single socket owned by the Events tab; a second socket would double-register the member. |
| n/a | Any `/ui/api` route | Presence is WS-only — there is no HTTP API to proxy. |

## Surprises

- **Presence has no HTTP API at all.** Unlike every prior UI cluster (which
  mounted tested handlers under `/ui/api`), presence is purely realtime — a
  `PresenceHub` fanning `presence_snapshot` frames over `/ws/subscribe`. So
  this cluster is the first pure front-end render with zero Rust change.
- **The data was already on the wire.** The subscribe already sent
  `member_id` and the frames were being dumped raw into the events log;
  surfacing presence was just a render of bytes already arriving.

## Decisions

- **Reflect the single Events-tab socket** rather than open a dedicated
  presence socket — avoids double-registering the operator in the hub.
- **No backend change.** The smallest correct surface; the protocol is
  already covered by the `presence_ws` e2e.

## Capability table extension

| Capability | Where |
|------------|-------|
| Live presence roster + online/away in the `/ui` (over the WS) | `static/index.html` (`renderPresence`/`setPresence`) |

## Risks identified + still open

- **JS behavior inspection-verified** (no browser) — standing UI limit; the
  `ui_js_contract` guard covers references, the `presence_ws` e2e covers the
  protocol.

## Forward look

Presence (140) joins reactions/pins/DMs/group-DMs/operator-console as
surfaced UI. The main remaining unsurfaced collaboration feature is slash
commands (`slash_commands.rs`); reassess against [[Open Work]] before
opening 141.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].

# Cluster 135.0 retro — Pins in the thread view

> Tag **`v135.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.

## What shipped

- **`/ui/api/threads/:tid/pins`** (session-gated): `GET` (read router) +
  `POST`/`DELETE` (write router), reusing the existing tested
  `list_pins`/`pin_message`/`unpin_message` handlers.
- **Pin affordance in `index.html`**: `loadMessages` loads the thread's pins
  into a `pinnedIds` set; each message meta shows a 📌 pin/unpin toggle that
  reflects pinned state and flips it (`togglePin` POST/DELETE
  `{message_id, member_id}`); `stopPropagation` keeps it from opening the edit
  panel.

## What was deferred / not covered

| To | What | Why |
|----|------|-----|
| Future | A dedicated pinned-messages panel | The per-message toggle + styling suffice; a pins-only view can follow. |
| n/a | `/ui/api` pin backend test | Handlers + `/ui/api` middleware are each already covered; new routes wire tested pieces. |

## Surprises

- **Pin + unpin share one body type** (`PinMessage { message_id, member_id }`),
  so the toggle is symmetric — POST vs DELETE on the same path with the same
  payload, branched on current pinned state.

## Decisions

- **Per-message toggle over a pins panel.** Lowest-friction; the pinned set is
  loaded alongside messages so state is always reflected.
- **Reuse handlers under `/ui/api`** (as with reactions) — no new backend logic.

## Capability table extension

| Capability | Where |
|------------|-------|
| Pin/unpin in the `/ui` console (per-message toggle) | `static/index.html`, `/ui/api/threads/:tid/pins` |

## Risks identified + still open

- **JS behavior inspection-verified** (no browser) — standing UI limit; guard
  covers references, `reactions_pins_e2e` covers the API.

## Forward look

Next UI clusters: **136** group DMs, **137** operator console.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].

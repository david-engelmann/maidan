# Cluster 135.0 — Pins in the thread view

**Theme:** Pin/unpin messages in the `/ui` console, over the already-shipped
pins API. Same pattern as reactions (134).

**Ladder:** Post-gate — **Phase XXIV**, tag **`v135.0.0`**, no new gate tag.

---

## Scope

| Layer | Deliverable |
|-------|-------------|
| **Routes (app.rs)** | `/ui/api/threads/:tid/pins` — `GET` (read) + `POST`/`DELETE` (write), reusing `list_pins`/`pin_message`/`unpin_message`. |
| **UI (index.html)** | `loadMessages` loads the thread's pins into `pinnedIds`; each message meta gets a 📌 pin/unpin toggle that reflects + flips state. |

## Non-goals

- A dedicated "pinned messages" panel — the per-message toggle + pinned styling
  is enough for the operator surface; a pins-only view can follow if wanted.
- A dedicated `/ui/api` pin backend test — handlers (reactions_pins_e2e) +
  middleware (existing routes) are each already covered.

## PR ladder (actual)

| # | Title |
|---|--------|
| 135.0.1 | `feat(ui): pin/unpin messages in the thread view` (#360) |
| 135.0.retro | `docs(retro): Cluster 135.0 + v135.0.0 tag prep` |

## Exit criteria

- Pins render + toggle in the UI; routes wired under `/ui/api`; guard green — **met**.
- `v135.0.0` tagged after retro.

## Verification & limits

- `ui_js_contract` guard validates the new JS; `fmt`/`clippy` clean. Per the UI
  track's standing limit, JS *behavior* is inspection-verified (no browser).

## References

- [[Retros/Cluster 135.0]]; `static/index.html` (`loadPins`/`togglePin`), `app.rs`

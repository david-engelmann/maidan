# Cluster 134.0 — Reactions in the operator UI

**Theme:** First UI feature on the repaired + guarded `/ui` base (133): an
emoji-reaction affordance on messages, over the already-shipped reactions API.

**Ladder:** Post-gate — **Phase XXIV**, tag **`v134.0.0`**, no new gate tag.

---

## Scope

| Layer | Deliverable |
|-------|-------------|
| **Routes (app.rs)** | Mount the existing reaction handlers under `/ui/api` (session-gated): `GET` (read router) + `POST`/`DELETE` (write router). No new handler logic. |
| **UI (index.html)** | Per-message reactions row: aggregated emoji chips + counts (own highlighted), quick-add buttons, click-to-toggle; `stopPropagation` so it doesn't open the row's edit. |

## Non-goals

- A full emoji picker — quick-reactions (👍 ❤️ ✅ 🎉 👀) cover the common case;
  arbitrary emoji is still possible via the bearer API.
- A dedicated `/ui/api` reaction backend test — the handlers (reactions_pins_e2e)
  and the `/ui/api` middleware (existing routes) are each already covered.

## PR ladder (actual)

| # | Title |
|---|--------|
| 134.0.1 | `feat(ui): emoji reactions on messages` (#358) |
| 134.0.retro | `docs(retro): Cluster 134.0 + v134.0.0 tag prep` |

## Exit criteria

- Reactions render + toggle in the UI; routes wired under `/ui/api`; guard green — **met**.
- `v134.0.0` tagged after retro.

## Verification & limits

- `ui_js_contract` guard validates the new JS helpers are defined and reference
  nothing undefined; `fmt`/`clippy` clean.
- Per the UI track's standing limit, the JS *behavior* is inspection-verified
  (no browser in CI); the backend reuses tested handlers + middleware.

## References

- [[Retros/Cluster 134.0]]; `static/index.html` (`loadReactions`/`toggleReaction`), `app.rs`

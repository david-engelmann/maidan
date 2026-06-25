# Cluster 134.0 retro — Reactions in the operator UI

> Tag **`v134.0.0`**. Phase XXIV (post-gate hardening). No new gate tag. First
> UI *feature* cluster (after 133 repaired the foundation).

## What shipped

- **`/ui/api` reaction routes** (session-gated): `GET` on the read router,
  `POST`/`DELETE` on the write router — mounting the existing, tested reaction
  handlers (no new handler logic). Bearer mode uses the top-level routes.
- **Reactions affordance in `index.html`**: each message shows aggregated emoji
  chips with counts (your own highlighted), quick-add buttons (👍 ❤️ ✅ 🎉 👀),
  and click-to-toggle; `stopPropagation` keeps reacting from opening the row's
  edit panel. Minimal chip CSS.

## What was deferred / not covered

| To | What | Why |
|----|------|-----|
| Future | Full emoji picker | Quick-reactions cover the common case; arbitrary emoji via the API. |
| n/a | Dedicated `/ui/api` reaction backend test | Handlers (reactions_pins_e2e) + `/ui/api` middleware (existing routes) are each already covered; the new routes are wiring of tested pieces. |

## Surprises

- **The feature was almost entirely wiring + JS.** Because the reaction backend
  already existed and 133 fixed the write helpers, "add reactions to the UI" was
  two route lines + a render function — the value of doing the foundation repair
  first.

## Decisions

- **Reuse handlers under `/ui/api`.** No new backend logic — the session-gated
  routes mount the same handlers, so reactions inherit the existing auth + tests.
- **Quick-reactions over a picker.** Lower-effort, covers the common case;
  arbitrary emoji remains available via the bearer API.

## Capability table extension

| Capability | Where |
|------------|-------|
| Emoji reactions in the `/ui` console (chips, quick-add, toggle) | `static/index.html`, `/ui/api/messages/:mid/reactions` |

## Risks identified + still open

- **JS behavior is inspection-verified** (no browser in CI) — the standing UI
  limit; the guard covers reference bugs, `reactions_pins_e2e` covers the API.
- **Per-message reaction fetch** on message load (one request each) — fine at
  operator scale; could batch if threads grow large.

## Forward look

Next UI clusters: **135** pins, **136** group DMs, **137** operator console.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]].

# Cluster 251.0 retro — the inbox gets a face

> Tag **`v251.0.0`**. Phase XXIV (post-gate hardening). No new gate tag.
> **Program C (notifications & reach), part 15** — Arc I.

## What shipped

- A "Notifications" tab in the `/ui`: lists the signed-in member's notifications with
  an unread-count badge, per-item "Mark read", a "Mark all read" button, and an
  "unread only" filter — over four new `/ui/api/members/:id/notifications*` routes that
  reuse the Cluster-239 handlers under the session middleware.

The whole notification system (ledger → router → prefs/follows → email) now has a
human face.

## Surprises / decisions

- **No new handlers, no bijection churn.** The `/ui/api` routes proxy straight to the
  239 handlers; because `/ui/api` is a curated capability-map subset and absent from
  OpenAPI, adding routes there needs no capability-map/OpenAPI entries (unlike the main
  API). The enforcement is already proven by `notifications_inbox_e2e`. So this cluster
  is a thin route layer + JS — the leanest way to surface an existing capability.
- **`sessionMemberId` makes self-only just work.** The 239 handlers are self-only for
  sessions (`ensure_acting_member`); the `/ui` already knows the signed-in member, so
  the tab passes it as `:id` and the guard is satisfied by construction — a session
  user can only ever load their own inbox.
- **Only real helpers, so the guard stays green.** The `/ui` has no browser test; the
  `ui_js_contract` static guard is the net for the classic "called an undefined helper"
  bug. Sticking to the established `uiReadPath` / `apiWritePath` / `headers` /
  `setStatus` / `requireAuthForWrite` helpers (rather than inventing an `authHeaders`)
  kept the guard passing first try.

## Capability table extension

| Change | Where |
|--------|-------|
| `/ui/api/members/:id/notifications*` routes + Notifications tab/panel/JS | `app.rs`, `static/index.html` |

## Risks identified + still open

- No browser test (structural to the `/ui`); mitigated by the JS contract guard + the
  tested backend handlers.

## Forward look

Per the plan (user: "do /ui then digests + presence-aware routing"), next is the
backend remainder of Arc I: scheduled **digests** + unread rollups, and
**presence-aware routing** — which first needs durable `last_seen` (presence is
in-memory only today). Then the optional MCP email tools for parity, and **Program D
(scale & durability)**.

## Acknowledgements

Solo maintainer cluster; admin-merge per [[Operations]]. Continues [[Retros/Cluster 250.0]].

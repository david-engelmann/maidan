# Cluster 251.0 — /ui notification center

> **Program C (notifications & reach), part 15** — Arc I. Phase XXIV post-gate
> hardening. Tag **`v251.0.0`**. No new gate tag.

## Goal

Surface the notification system to humans: a "Notifications" tab in the `/ui` that
lists the signed-in member's notifications, shows the unread count, and lets them
mark one / all read — reading the Cluster-239 handlers through the session-authed
`/ui/api` proxy.

## Scope

| Change | Where |
|--------|-------|
| `/ui/api/members/:id/notifications` + `…/unread-count` (read) + `…/:nid/read` + `…/read-all` (write) — reuse the Cluster-239 handlers under the `/ui` session middleware | `app.rs` |
| A "Notifications" tab + panel + JS (`loadNotifications` / `renderNotifications` / `markNotificationRead` / `markAllNotificationsRead`) | `static/index.html` |

## Design decisions

- **Reuse the tested handlers via `/ui/api`, no new handlers.** The `/ui` proxies to
  the same `list_member_notifications` / `member_unread_notification_count` /
  `mark_member_notification_read` / `mark_all_member_notifications_read` handlers
  (Cluster 239) under the session middleware — reads on `ui_api_read`, writes on
  `ui_api_write`. Their `ensure_acting_member` self-only check is satisfied because the
  session's member equals the `:id` the JS passes (`sessionMemberId`).
- **No capability-map / OpenAPI churn.** `/ui/api` routes are a curated subset in the
  capability map (many — reactions, pins, dm — aren't listed) and are absent from
  OpenAPI, so a new `/ui/api` route needs no bijection entry. Enforcement is proven by
  the main-API `notifications_inbox_e2e` (239), which exercises the same handlers.
- **`sessionMemberId` is "me".** The JS already tracks the signed-in member; the tab
  passes it as `:id`, so a session user sees only their own inbox (matching the
  self-only guard). Reads via `uiReadPath`, writes via `apiWritePath` — the same
  helpers every other `/ui` feature uses, so the `ui_js_contract` guard stays green.

## Non-goals / deferred

- A live unread-badge on the tab button (the badge lives in the panel header, refreshed
  on load / mark) — a nicety, not needed for the MVP.
- Digests, presence-aware routing, optional MCP email tools (rest of Arc I).

## Risks

- The `/ui` has no browser test in CI; the `ui_js_contract` static guard (Cluster 133)
  catches undefined-helper bugs, and every function/​helper the new JS calls is defined
  — the guard passes. Backend logic is the already-tested 239 handlers.

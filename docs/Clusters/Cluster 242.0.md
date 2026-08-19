# Cluster 242.0 — mute-aware router + preferences REST

> **Program C (notifications & reach), part 6** — Arc H. Phase XXIV post-gate
> hardening. Tag **`v242.0.0`**. No new gate tag.

## Goal

Make the Cluster-241 mute preference actually do something, and let members set it: the
notification router consults `is_notification_muted` before writing, and REST exposes
set/list of a member's preferences.

## Scope

| Change | Where |
|--------|-------|
| Router honors mute — `route_event` skips a muted `(member, kind)` + a suppressed metric | `notification_router.rs`, `metrics.rs` |
| `PUT`/`GET /members/:id/notification-prefs` — set (upsert) / list; `workspace:read`, self-only for sessions | `routes/member.rs`, `app.rs`, `dto.rs` |
| Full new-route preflight (OpenAPI + capability-map + matrix PUT body clause) | `openapi/*`, `contracts/http-capability-map.json`, `http_capability_matrix_e2e.rs` |

## Design decisions

- **The mute check is one store call in `route_event`.** Because Cluster 241 put the
  "absent row → not muted" default in `is_notification_muted`, the router change is a
  single guard: `if is_notification_muted(member, kind) { record suppressed; return }`.
  The suppression is metered separately (`maidan_notifications_suppressed_total{reason}`,
  `reason=muted`) so muted-vs-written is observable.
- **Preferences are self-config, so `workspace:read` + self-only.** Setting your own
  mute is configuring your own state, not writing workspace content — the same cap +
  `ensure_acting_member` model the inbox mark-read routes use (Cluster 239). A session
  caller manages only their own prefs; a bearer is the act-as-any orchestrator.
- **Router + REST in one cluster; MCP next.** The router guard is ~5 lines and the
  REST surface is 2 routes, so shipping them together delivers the whole mute feature
  over REST in one slice; the MCP tools follow in 243 (the established REST-then-MCP
  split, 239 → 240).

## Non-goals / deferred

- **MCP** `set_notification_pref` / `list_notification_prefs` (Cluster 243).
- **Follows / subscription** (channel + thread follow → notify on activity) — a later
  Arc-H cluster.

## Risks

- New-route preflight (the standing multi-file gotcha) — the PUT carries a body, so
  it needs the matrix body clause (extractor 422s before `cap()`); covered by
  `openapi_e2e` + `http_capability_matrix_e2e`.

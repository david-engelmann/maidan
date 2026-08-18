# Cluster 239.0 — REST unified inbox

> **Program C (notifications & reach), part 3** — Arc G. Phase XXIV post-gate
> hardening. Tag **`v239.0.0`**. No new gate tag.

## Goal

Give members a way to *read* the per-recipient ledger the router (238) fills: a REST
unified inbox — list notifications, the unread badge, mark one read, mark all read.

## Scope

| Change | Where |
|--------|-------|
| `GET /members/:id/notifications` (list; `unread_only`, `limit`) + `GET …/unread-count` + `POST …/:nid/read` + `POST …/read-all` — all `workspace:read`, **self-only** for sessions | `routes/member.rs`, `app.rs` |
| `mark_notification_read` becomes recipient-scoped — `(member_id, id)` | `store.rs`, `store/*/notifications.rs`, `store/*/mod.rs` |
| DTOs `ListNotificationsQuery` / `UnreadCount` / `MarkAllRead`; full new-route preflight (OpenAPI + capability-map + matrix `{nid}`) | `dto.rs`, `openapi/*`, `contracts/http-capability-map.json`, `http_capability_matrix_e2e.rs` |

## Design decisions

- **Self-only for sessions, act-as-any for bearers.** A member reads their *own*
  inbox, so a `/ui` session caller is guarded by `ensure_acting_member` (path `:id`
  must equal the session's member). A bearer token is the orchestrator (act-as-any) —
  the Cluster-202/203 model. This is the correct model the *older* mention/inbox
  routes never got (they're `workspace:read`-only, readable by any workspace member —
  logged as a follow-up, not retrofitted here).
- **`mark_notification_read` is recipient-scoped in the store, not just the route.**
  The mark route already guards self-only, but scoping the UPDATE to `(member_id,
  id)` makes it safe-by-construction: even a bearer can't mark a notification that
  isn't the path member's, and a bad/foreign id returns `404`. A re-mark preserves
  the first-read time (`COALESCE`).
- **Mark returns the fresh badge.** `POST …/:nid/read` and `…/read-all` return the
  updated `UnreadCount` / `{cleared}` so a UI updates the badge without a follow-up
  request.
- **Bodyless POSTs.** Mark-read and read-all take no request body (the ids are in the
  path), so there's no extractor-before-`cap()` ordering hazard — no matrix body
  clause needed, only the `{nid}` path substitution.

## Non-goals / deferred

- **MCP** `list_notifications` / `mark_notification_read` / `get_unread_count` + a
  **`wait_for_notification`** long-poll (Cluster 240, closes Arc G).
- Retrofitting self-only onto the legacy mention/inbox routes (follow-up).

## Risks

- New-route preflight is the standing multi-file gotcha (OpenAPI paths + components,
  capability-map, matrix substitution) — covered by `openapi_e2e` (bijection) +
  `http_capability_matrix_e2e`, both green.

# Cluster 241.0 — notification mute-preferences foundation (opens Arc H)

> **Program C (notifications & reach), part 5** — opens **Arc H (preferences +
> subscription)**. Phase XXIV post-gate hardening. Tag **`v241.0.0`**. No new gate
> tag.

## Goal

Open Arc H with the zero-blast-radius foundation for the routing brain: a per-member,
per-event-kind **mute** preference the notification router (238) will consult before
writing. This cluster lands the store; the router change + REST/MCP management
follow.

## Scope

| Change | Where |
|--------|-------|
| `maidan_notification_prefs` table (pg 0044 / sqlite 0043; PK `(member_id, kind)`, `muted` flag) | `migrations/*`, `migrate.rs` |
| `NotificationPref` model | `maidan-types/src/models.rs` |
| Store — `set_notification_pref` (upsert), `list_notification_prefs`, `is_notification_muted` (router query), both backends | `store.rs`, `store/{sqlite,postgres}/notification_prefs.rs`, `store/*/mod.rs` |

## Design decisions

- **A row per `(member, EventKind)` with a `muted` flag, absent = notify.** The
  notification's `kind` is the triggering `EventKind` (Cluster 237), so muting keys on
  the same vocabulary — no new enum. A `muted` bool (rather than a presence-only mute
  table) keeps the row explicit (`set(..., false)` records "explicitly on") and leaves
  room for the table to grow more preference columns without a schema shuffle.
- **`is_notification_muted` is the router's question, answered in the store.** The
  router (242) will call it per candidate `(member, kind)`; putting the "absent =
  false" default in one store method keeps the router change to a single guard.
- **Foundation only.** A new table + module; zero existing paths change (Cluster
  159 / 230 pattern). The router still notifies unconditionally until 242.

## Non-goals / deferred (the rest of Arc H)

- **Router wiring** (Cluster 242) — `route_event` skips a muted `(member, kind)`.
- **Follows / subscription** (channel + thread follow → notify on activity) — a
  later Arc-H cluster.
- **REST + MCP** management of prefs — a later Arc-H cluster.

## Risks

- Migration registration (the standing gotcha) — covered by the both-backend store
  test + `dialect_parity` / `concurrent_migrations`.
